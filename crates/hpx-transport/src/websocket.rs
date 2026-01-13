//! High-performance WebSocket client implementation.
//!
//! This module provides a robust, actor-based WebSocket client with support for:
//! - Automatic reconnection with exponential backoff
//! - Subscription management (Pub/Sub)
//! - Exchange-specific message routing via `ExchangeHandler` trait
//! - Thread-safe message sending
//!
//! # Architecture
//!
//! The implementation follows the Actor model:
//! - `WebSocketClient`: A lightweight handle to the actor, used by the application.
//! - `WebSocketActor`: The background task managing the connection, state, and message routing.
//!
//! Messages are routed based on topics extracted by the `ExchangeHandler`.

use std::{collections::HashMap, time::Duration};

use futures_util::StreamExt;
use hpx::ws::{WebSocket, WebSocketWrite, message::Message};
use opentelemetry::{KeyValue, global, metrics::Counter};
use serde::Serialize;
use tokio::{
    sync::{broadcast, mpsc, oneshot},
    time::sleep,
};
use tracing::{error, info, warn};

use crate::error::{TransportError, TransportResult};

/// Trait for exchange-specific WebSocket logic.
pub trait ExchangeHandler: Send + Sync + 'static {
    /// Process incoming message and return routing info
    /// Returns (topic, processed_message) or None if message should be ignored
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

    /// Decode binary data to text (e.g., decompress)
    fn decode_binary(&self, data: &[u8]) -> Option<String> {
        String::from_utf8(data.to_vec()).ok()
    }

    /// Extract topic from text message
    fn get_topic(&self, message: &str) -> Option<String>;

    /// Build a subscribe message for the given topic.
    fn build_subscribe_message(&self, topic: &str) -> Message;

    /// Build an unsubscribe message for the given topic.
    fn build_unsubscribe_message(&self, topic: &str) -> Message;

    /// Called when the connection is established.
    /// Returns a list of messages to send immediately (e.g. authentication).
    fn on_open(&self) -> Vec<Message> {
        vec![]
    }
}

/// Configuration for the WebSocket client.
#[derive(Debug, Clone)]
pub struct WebSocketConfig {
    pub url: String,
    pub reconnect_initial_delay: Duration,
    pub reconnect_max_delay: Duration,
    pub reconnect_backoff_factor: f64,
    pub channel_capacity: usize,
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            reconnect_initial_delay: Duration::from_millis(100),
            reconnect_max_delay: Duration::from_secs(30),
            reconnect_backoff_factor: 2.0,
            channel_capacity: 1024,
        }
    }
}

impl WebSocketConfig {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Default::default()
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

/// The user-facing WebSocket client.
#[derive(Debug, Clone)]
pub struct WebSocketClient {
    cmd_tx: mpsc::UnboundedSender<Command>,
}

impl WebSocketClient {
    /// Create and start a new WebSocket client.
    pub async fn new<H>(config: WebSocketConfig, handler: H) -> TransportResult<Self>
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
            .map_err(|_| TransportError::Internal {
                message: "WebSocket actor is closed".to_string(),
            })?;

        reply_rx.await.map_err(|_| TransportError::Internal {
            message: "Failed to receive subscription channel".to_string(),
        })
    }

    /// Unsubscribe from a topic.
    pub async fn unsubscribe(&self, topic: impl Into<String>) -> TransportResult<()> {
        self.cmd_tx
            .send(Command::Unsubscribe {
                topic: topic.into(),
            })
            .map_err(|_| TransportError::Internal {
                message: "WebSocket actor is closed".to_string(),
            })?;
        Ok(())
    }

    /// Send a raw message to the server.
    pub async fn send(&self, message: Message) -> TransportResult<()> {
        self.cmd_tx
            .send(Command::Send(message))
            .map_err(|_| TransportError::Internal {
                message: "WebSocket actor is closed".to_string(),
            })?;
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
    active_topics: HashMap<String, ()>, // Track active subscriptions to resubscribe on reconnect
    should_stop: bool,

    // Connection health monitoring
    write: Option<WebSocketWrite>,
    last_pong: Option<std::time::Instant>,

    // Metrics
    messages_sent: Counter<u64>,
    messages_received: Counter<u64>,
    errors: Counter<u64>,
}

impl<H: ExchangeHandler> WebSocketActor<H> {
    fn new(config: WebSocketConfig, handler: H, cmd_rx: mpsc::UnboundedReceiver<Command>) -> Self {
        let meter = global::meter("longtrader-transport");
        Self {
            config,
            handler,
            cmd_rx,
            subscriptions: HashMap::new(),
            active_topics: HashMap::new(),
            should_stop: false,
            write: None,
            last_pong: None,
            messages_sent: meter
                .u64_counter("transport.ws.messages_sent")
                .with_description("Total number of WebSocket messages sent")
                .build(),
            messages_received: meter
                .u64_counter("transport.ws.messages_received")
                .with_description("Total number of WebSocket messages received")
                .build(),
            errors: meter
                .u64_counter("transport.ws.errors")
                .with_description("Total number of WebSocket errors")
                .build(),
        }
    }

    async fn connect(url: &str) -> TransportResult<WebSocket> {
        let client = hpx::Client::builder()
            .build()
            .map_err(|e| TransportError::Config {
                message: format!("Failed to build client: {}", e),
            })?;

        let req = client.get(url);
        let ws_req = hpx::ws::WebSocketRequestBuilder::new(req);

        let resp = ws_req.send().await.map_err(|e| {
            TransportError::Network(Box::new(crate::error::NetworkError::Hpx { source: e }))
        })?;

        resp.into_websocket().await.map_err(|e| {
            TransportError::Network(Box::new(crate::error::NetworkError::Hpx { source: e }))
        })
    }

    async fn run(mut self) {
        let mut reconnect_delay = self.config.reconnect_initial_delay;

        loop {
            info!("Connecting to {}", self.config.url);

            match Self::connect(&self.config.url).await {
                Ok(ws_stream) => {
                    info!("Connected to {}", self.config.url);
                    reconnect_delay = self.config.reconnect_initial_delay; // Reset backoff

                    if let Err(e) = self.handle_connection(ws_stream).await {
                        self.errors
                            .add(1, &[KeyValue::new("type", "connection_error")]);
                        error!("Connection error: {}", e);
                    } else {
                        if self.should_stop {
                            info!("Client closed by user.");
                            break;
                        }
                        info!("Connection closed by server, reconnecting...");
                    }
                }
                Err(e) => {
                    self.errors
                        .add(1, &[KeyValue::new("type", "connect_error")]);
                    error!("Failed to connect: {}", e);
                }
            }

            // Reconnect logic
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

        // Setup ping timer
        let mut ping_timer = tokio::time::interval(Duration::from_secs(30));
        ping_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Send on_open messages (e.g. auth)
        for msg in self.handler.on_open() {
            self.send_message(msg).await?;
        }

        // Resubscribe to active topics
        let topics: Vec<String> = self.active_topics.keys().cloned().collect();
        for topic in topics {
            let msg = self.handler.build_subscribe_message(&topic);
            self.send_message(msg).await?;
        }

        loop {
            tokio::select! {
                // Handle incoming WebSocket messages
                msg = read.next() => {
                    match msg {
                        Some(Ok(msg)) => {
                            self.process_message(msg).await;
                        }
                        Some(Err(e)) => {
                            return Err(TransportError::Network(Box::new(
                                crate::error::NetworkError::Hpx { source: e }
                            )));
                        }
                        None => {
                            info!("WebSocket stream ended");
                            return Ok(());
                        }
                    }
                }

                // Periodic ping
                _ = ping_timer.tick() => {
                    if let Err(e) = self.send_message(Message::Ping(bytes::Bytes::from(vec![]))).await {
                        error!("Failed to send ping: {}", e);
                        return Err(e);
                    }

                    // Check for timeout
                    if self.last_pong.is_some_and(|last_pong| last_pong.elapsed() > Duration::from_secs(90)) {
                        error!("Ping timeout - no pong received");
                        return Err(TransportError::Timeout {
                            duration: Duration::from_secs(90),
                        });
                    }
                }

                // Handle commands from Client
                Some(cmd) = self.cmd_rx.recv() => {
                    self.handle_command(cmd).await?;
                }
            }
        }
    }

    async fn send_message(&mut self, msg: Message) -> TransportResult<()> {
        if let Some(write) = self.write.as_mut() {
            match write.send(msg).await {
                Ok(_) => {
                    self.messages_sent.add(1, &[]);
                    Ok(())
                }
                Err(e) => {
                    self.errors.add(1, &[KeyValue::new("type", "send_error")]);
                    Err(TransportError::Network(Box::new(
                        crate::error::NetworkError::Hpx { source: e },
                    )))
                }
            }
        } else {
            self.errors
                .add(1, &[KeyValue::new("type", "writer_unavailable")]);
            Err(TransportError::Internal {
                message: "WebSocket writer not available".to_string(),
            })
        }
    }

    async fn handle_command(&mut self, cmd: Command) -> TransportResult<()> {
        match cmd {
            Command::Subscribe { topic, reply_tx } => {
                let rx = self.get_or_create_subscription(&topic);
                let _ = reply_tx.send(rx);

                let msg = self.handler.build_subscribe_message(&topic);
                self.send_message(msg).await?;
                self.active_topics.insert(topic, ());
            }
            Command::Unsubscribe { topic } => {
                self.active_topics.remove(&topic);
                let msg = self.handler.build_unsubscribe_message(&topic);
                self.send_message(msg).await?;
            }
            Command::Send(msg) => {
                self.send_message(msg).await?;
            }
            Command::Close => {
                self.should_stop = true;
                self.send_message(Message::Close(None)).await?;
            }
        }
        Ok(())
    }

    async fn process_message(&mut self, msg: Message) {
        self.messages_received.add(1, &[]);
        match msg {
            Message::Text(text) => {
                if let Some(topic) = self.handler.get_topic(text.as_str()) {
                    self.dispatch(&topic, Message::Text(text));
                }
            }
            Message::Binary(data) => {
                // Try to decode as text (for compressed messages)
                if let Some(text) = self.handler.decode_binary(&data) {
                    if let Some(topic) = self.handler.get_topic(&text) {
                        self.dispatch(&topic, Message::Text(text.into()));
                    }
                } else {
                    warn!("Received unhandled binary message");
                }
            }
            Message::Ping(_) | Message::Pong(_) => {
                // Auto-handled by fastwebsockets usually if configured, but we can handle here too
                if let Message::Pong(_) = msg {
                    self.last_pong = Some(std::time::Instant::now());
                }
            }
            Message::Close(_) => {
                info!("Received close frame");
            }
        }
    }

    fn dispatch(&mut self, topic: &str, msg: Message) {
        if let Some(tx) = self.subscriptions.get(topic) {
            // Ignore send errors (no subscribers)
            let _ = tx.send(msg);
        }
    }

    fn get_or_create_subscription(&mut self, topic: &str) -> broadcast::Receiver<Message> {
        if let Some(tx) = self.subscriptions.get(topic) {
            tx.subscribe()
        } else {
            let (tx, rx) = broadcast::channel(self.config.channel_capacity);
            self.subscriptions.insert(topic.to_string(), tx);
            rx
        }
    }
}
