//! Connection actor for WebSocket lifecycle management.
//!
//! The actor runs in a background task and handles:
//! - Connection establishment and maintenance
//! - Automatic reconnection with exponential backoff
//! - Heartbeat (ping/pong) management
//! - Message routing (responses to pending requests, updates to subscriptions)
//! - Command processing from the client API

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::Bytes;
use futures_util::StreamExt;
use hpx::ws::{WebSocket, WebSocketRead, WebSocketWrite, message::Message};
use rand::Rng;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};

use super::{
    config::WsConfig,
    pending::PendingRequestStore,
    protocol::{ProtocolHandler, WsMessage},
    subscription::SubscriptionStore,
    types::{MessageKind, RequestId, Topic},
};
use crate::error::{TransportError, TransportResult};

// ============================================================================
// Command Types (Task 3.1)
// ============================================================================

/// Commands sent from the client API to the connection actor.
pub enum ActorCommand {
    /// Subscribe to topics.
    Subscribe {
        /// Topics to subscribe to.
        topics: Vec<Topic>,
        /// Channel to send the result.
        reply_tx: oneshot::Sender<TransportResult<()>>,
    },
    /// Unsubscribe from topics.
    Unsubscribe {
        /// Topics to unsubscribe from.
        topics: Vec<Topic>,
    },
    /// Send a message without expecting a response.
    Send {
        /// Message to send.
        message: WsMessage,
    },
    /// Send a request and wait for a response.
    Request {
        /// Message to send.
        message: WsMessage,
        /// Request ID for correlation.
        request_id: RequestId,
        /// Channel to send the response.
        reply_tx: oneshot::Sender<TransportResult<String>>,
        /// Optional timeout override.
        timeout: Option<Duration>,
    },
    /// Close the connection gracefully.
    Close,
}

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

// ============================================================================
// Connection Actor (Tasks 3.2 - 3.9)
// ============================================================================

/// The connection actor manages a WebSocket connection lifecycle.
///
/// It runs as a background task and communicates with [`super::WsClient`]
/// via channels.
pub struct ConnectionActor<H: ProtocolHandler> {
    /// Configuration.
    config: Arc<WsConfig>,
    /// Protocol handler for exchange-specific logic.
    handler: H,
    /// Channel for receiving commands from the client.
    cmd_rx: mpsc::Receiver<ActorCommand>,
    /// Pending request store.
    pending_requests: Arc<PendingRequestStore>,
    /// Subscription store.
    subscriptions: Arc<SubscriptionStore>,
    /// Current connection state.
    state: ConnectionState,
    /// WebSocket write half (when connected).
    ws_write: Option<WebSocketWrite>,
    /// Last time we received a pong.
    last_pong: Instant,
    /// Current reconnection attempt count.
    reconnect_attempt: u32,
}

impl<H: ProtocolHandler> ConnectionActor<H> {
    /// Create a new connection actor.
    pub fn new(
        config: Arc<WsConfig>,
        handler: H,
        cmd_rx: mpsc::Receiver<ActorCommand>,
        pending_requests: Arc<PendingRequestStore>,
        subscriptions: Arc<SubscriptionStore>,
    ) -> Self {
        Self {
            config,
            handler,
            cmd_rx,
            pending_requests,
            subscriptions,
            state: ConnectionState::Disconnected,
            ws_write: None,
            last_pong: Instant::now(),
            reconnect_attempt: 0,
        }
    }

    /// Main entry point - run the actor until closed.
    pub async fn run(mut self) {
        info!(url = %self.config.url, "Starting WebSocket actor");

        loop {
            match &self.state {
                ConnectionState::Disconnected => {
                    self.state = ConnectionState::Connecting;
                }
                ConnectionState::Connecting | ConnectionState::Reconnecting { .. } => {
                    match self.try_connect().await {
                        Ok(ws) => {
                            // Connection successful
                            self.reconnect_attempt = 0;

                            if self.config.auth_on_connect {
                                self.state = ConnectionState::Authenticating;
                            } else {
                                self.state = ConnectionState::Ready;
                            }

                            // Run the connection loop
                            if let Err(e) = self.run_connection(ws).await {
                                warn!(error = %e, "Connection error");
                                self.initiate_reconnect();
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "Connection failed");
                            if self.should_give_up() {
                                error!("Max reconnection attempts exceeded");
                                self.pending_requests
                                    .clear_with_error("Max reconnection attempts exceeded");
                                self.state = ConnectionState::Closed;
                                continue;
                            }
                            self.wait_before_reconnect().await;
                        }
                    }
                }
                ConnectionState::Authenticating
                | ConnectionState::Connected
                | ConnectionState::Ready => {
                    // These states are handled within run_connection
                    // If we're here, it means the connection loop exited
                    if self.state != ConnectionState::Closing
                        && self.state != ConnectionState::Closed
                    {
                        self.initiate_reconnect();
                    }
                }
                ConnectionState::Closing => {
                    self.disconnect().await;
                    self.state = ConnectionState::Closed;
                }
                ConnectionState::Closed => {
                    break;
                }
            }
        }

        info!("WebSocket actor stopped");
    }

    /// Attempt to establish a WebSocket connection.
    async fn try_connect(&mut self) -> TransportResult<WebSocket> {
        debug!(url = %self.config.url, "Connecting to WebSocket");

        let url = self.config.url.clone();
        let connect_result =
            tokio::time::timeout(self.config.connect_timeout, Self::connect_websocket(&url)).await;

        match connect_result {
            Ok(Ok(ws)) => {
                self.last_pong = Instant::now();
                self.state = ConnectionState::Connected;
                info!("WebSocket connected");
                Ok(ws)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(TransportError::timeout(self.config.connect_timeout)),
        }
    }

    /// Establish the WebSocket connection.
    ///
    /// This is a static async function to avoid capturing `&self` in the future,
    /// which would require `ConnectionActor` to be `Sync`.
    async fn connect_websocket(url: &str) -> TransportResult<WebSocket> {
        let client = hpx::Client::builder()
            .build()
            .map_err(|e| TransportError::config(format!("Failed to build client: {e}")))?;

        let req = client.get(url);
        let ws_req = hpx::ws::WebSocketRequestBuilder::new(req);

        let resp = ws_req.send().await.map_err(TransportError::Http)?;

        resp.into_websocket().await.map_err(TransportError::Http)
    }

    /// Run the connection loop with established WebSocket.
    async fn run_connection(&mut self, ws: WebSocket) -> TransportResult<()> {
        let (write, mut read) = ws.split();
        self.ws_write = Some(write);

        // Send on_connect messages
        for msg in self.handler.on_connect() {
            if let Err(e) = self.send_message_internal(&msg).await {
                warn!(error = %e, "Failed to send on_connect message");
            }
        }

        // Handle authentication if needed
        if self.state == ConnectionState::Authenticating {
            if !self.authenticate(&mut read).await? {
                warn!("Authentication failed, reconnecting");
                return Err(TransportError::auth("Authentication failed"));
            }
            self.state = ConnectionState::Ready;
        }

        // Resubscribe to all topics
        if let Err(e) = self.resubscribe_all().await {
            warn!(error = %e, "Failed to resubscribe topics");
        }

        // Run the ready loop using the same authenticated connection.
        self.run_ready_loop_internal(read).await
    }

    /// Disconnect the WebSocket.
    async fn disconnect(&mut self) {
        if let Some(ws) = self.ws_write.take() {
            use hpx::ws::message::CloseCode;
            let _ = ws.close(CloseCode::NORMAL, "").await;
        }
        self.pending_requests.clear_with_error("Connection closed");
    }

    /// Check if we should give up reconnecting.
    fn should_give_up(&self) -> bool {
        if let Some(max) = self.config.reconnect_max_attempts {
            self.reconnect_attempt >= max
        } else {
            false
        }
    }

    /// Calculate backoff delay with jitter.
    fn calculate_backoff(&self) -> Duration {
        let base = self.config.reconnect_initial_delay.as_secs_f64()
            * self
                .config
                .reconnect_backoff_factor
                .powi(i32::try_from(self.reconnect_attempt).unwrap_or(i32::MAX));

        let max = self.config.reconnect_max_delay.as_secs_f64();
        let capped = base.min(max);

        // Add jitter using rand
        let mut rng = rand::rng();
        let jitter = capped * self.config.reconnect_jitter * rng.random::<f64>();
        Duration::from_secs_f64(capped + jitter)
    }

    /// Wait before the next reconnection attempt.
    async fn wait_before_reconnect(&mut self) {
        let delay = self.calculate_backoff();
        debug!(
            delay_ms = delay.as_millis(),
            attempt = self.reconnect_attempt,
            "Waiting before reconnect"
        );
        tokio::time::sleep(delay).await;
        self.reconnect_attempt += 1;
        self.state = ConnectionState::Reconnecting {
            attempt: self.reconnect_attempt,
        };
    }

    /// Initiate reconnection.
    fn initiate_reconnect(&mut self) {
        self.ws_write = None;
        self.state = ConnectionState::Reconnecting {
            attempt: self.reconnect_attempt,
        };
    }

    /// Perform authentication.
    async fn authenticate(&mut self, read: &mut WebSocketRead) -> TransportResult<bool> {
        let auth_msg = match self.handler.build_auth_message() {
            Some(msg) => msg,
            None => return Ok(true), // No auth required
        };

        if let Err(e) = self.send_message_internal(&auth_msg).await {
            warn!(error = %e, "Failed to send auth message");
            return Ok(false);
        }

        // Wait for auth response with timeout
        let timeout = self.config.request_timeout;
        let start = Instant::now();

        while start.elapsed() < timeout {
            let read_timeout = timeout.saturating_sub(start.elapsed());

            match tokio::time::timeout(read_timeout, read.next()).await {
                Ok(Some(Ok(msg))) => {
                    if let Some(text) = self.message_to_text(&msg) {
                        if self.handler.is_auth_success(&text) {
                            info!("Authentication successful");
                            return Ok(true);
                        }
                        if self.handler.is_auth_failure(&text) {
                            error!("Authentication rejected");
                            return Ok(false);
                        }
                    }
                }
                Ok(Some(Err(e))) => {
                    warn!(error = %e, "Error reading auth response");
                    return Err(TransportError::websocket(e.to_string()));
                }
                Ok(None) => {
                    // Connection closed
                    return Err(TransportError::connection_closed(None));
                }
                Err(_) => {
                    warn!("Authentication timed out");
                    return Ok(false);
                }
            }
        }

        warn!("Authentication timed out");
        Ok(false)
    }

    /// Convert a Message to text if possible.
    fn message_to_text(&self, msg: &Message) -> Option<String> {
        match msg {
            Message::Text(text) => Some(text.to_string()),
            Message::Binary(data) => self.handler.decode_binary(data).ok(),
            _ => None,
        }
    }

    /// Resubscribe to all topics after reconnection.
    async fn resubscribe_all(&mut self) -> TransportResult<()> {
        let topics = self.subscriptions.get_all_topics();
        if topics.is_empty() {
            return Ok(());
        }

        debug!(count = topics.len(), "Resubscribing to topics");

        let request_id = self.handler.generate_request_id();
        let msg = self.handler.build_subscribe(&topics, request_id);
        self.send_message_internal(&msg).await?;

        Ok(())
    }

    /// The main event loop when the connection is ready.
    ///
    /// Uses the authenticated read half from the active WebSocket connection.
    async fn run_ready_loop_internal(&mut self, mut read: WebSocketRead) -> TransportResult<()> {
        let mut ping_interval = tokio::time::interval(self.config.ping_interval);
        let mut cleanup_interval = tokio::time::interval(self.config.pending_cleanup_interval);

        loop {
            tokio::select! {
                // Handle incoming commands
                cmd = self.cmd_rx.recv() => {
                    match cmd {
                        Some(cmd) => match cmd {
                            ActorCommand::Close => {
                                self.state = ConnectionState::Closing;
                                return Ok(());
                            }
                            ActorCommand::Subscribe { topics, reply_tx } => {
                                let result = self.handle_subscribe(topics).await;
                                let _ = reply_tx.send(result);
                            }
                            ActorCommand::Unsubscribe { topics } => {
                                self.handle_unsubscribe(topics).await;
                            }
                            ActorCommand::Send { message } => {
                                if let Err(e) = self.send_message_internal(&message).await {
                                    warn!(error = %e, "Failed to send message");
                                }
                            }
                            ActorCommand::Request { message, request_id, reply_tx, timeout } => {
                                self.handle_request(message, request_id, reply_tx, timeout).await;
                            }
                        },
                        None => {
                            info!("All command senders dropped; shutting down actor");
                            self.disconnect().await;
                            self.state = ConnectionState::Closed;
                            return Ok(());
                        }
                    }
                }

                // Read incoming messages
                result = read.next() => {
                    match result {
                        Some(Ok(msg)) => {
                            if let Err(e) = self.handle_message(msg).await {
                                warn!(error = %e, "Error handling message");
                            }
                        }
                        Some(Err(e)) => {
                            return Err(TransportError::websocket(e.to_string()));
                        }
                        None => {
                            // Connection closed by server
                            return Err(TransportError::connection_closed(None));
                        }
                    }
                }

                // Send ping
                _ = ping_interval.tick() => {
                    if let Err(e) = self.send_ping().await {
                        warn!(error = %e, "Failed to send ping");
                        return Err(e);
                    }

                    // Check pong timeout
                    if self.last_pong.elapsed() > self.config.pong_timeout {
                        warn!("Pong timeout");
                        return Err(TransportError::websocket("Pong timeout"));
                    }
                }

                // Cleanup stale requests
                _ = cleanup_interval.tick() => {
                    self.pending_requests.cleanup_stale_with_notify();
                }
            }
        }
    }

    /// Handle an incoming message.
    async fn handle_message(&mut self, msg: Message) -> TransportResult<()> {
        // Handle protocol-level messages
        match &msg {
            Message::Pong(_) => {
                self.last_pong = Instant::now();
                return Ok(());
            }
            Message::Ping(data) => {
                if let Some(ref mut ws) = self.ws_write {
                    let _ = ws.send(Message::Pong(data.clone())).await;
                }
                return Ok(());
            }
            Message::Close(_) => {
                return Err(TransportError::connection_closed(None));
            }
            _ => {}
        }

        let text = match self.message_to_text(&msg) {
            Some(t) => t,
            None => return Ok(()),
        };

        // Check for server ping
        if self.handler.is_server_ping(&text) {
            if let Some(pong) = self.handler.build_pong(text.as_bytes()) {
                self.send_message_internal(&pong).await?;
            }
            return Ok(());
        }

        // Check for pong response
        if self.handler.is_pong_response(&text) {
            self.last_pong = Instant::now();
            return Ok(());
        }

        // Check for reconnect request
        if self.handler.should_reconnect(&text) {
            return Err(TransportError::websocket("Server requested reconnect"));
        }

        // Classify and route the message
        match self.handler.classify_message(&text) {
            MessageKind::Response => {
                if let Some(request_id) = self.handler.extract_request_id(&text) {
                    self.pending_requests.resolve(&request_id, Ok(text));
                }
            }
            MessageKind::Update => {
                if let Some(topic) = self.handler.extract_topic(&text) {
                    self.subscriptions.publish(&topic, WsMessage::Text(text));
                }
            }
            MessageKind::System | MessageKind::Control => {
                debug!(message = text, "Received control message");
            }
            MessageKind::Unknown => {
                debug!(message = text, "Received unknown message type");
            }
        }

        Ok(())
    }

    /// Handle a subscribe command.
    async fn handle_subscribe(&mut self, topics: Vec<Topic>) -> TransportResult<()> {
        let mut new_topics = Vec::new();

        for topic in topics {
            let (_, is_new) = self.subscriptions.subscribe(topic.clone());
            if is_new {
                new_topics.push(topic);
            }
        }

        if !new_topics.is_empty() {
            let request_id = self.handler.generate_request_id();
            let msg = self.handler.build_subscribe(&new_topics, request_id);
            self.send_message_internal(&msg).await?;
        }

        Ok(())
    }

    /// Handle an unsubscribe command.
    async fn handle_unsubscribe(&mut self, topics: Vec<Topic>) {
        let mut removed_topics = Vec::new();

        for topic in topics {
            if self.subscriptions.unsubscribe(&topic) {
                removed_topics.push(topic);
            }
        }

        if !removed_topics.is_empty() {
            let request_id = self.handler.generate_request_id();
            let msg = self.handler.build_unsubscribe(&removed_topics, request_id);
            if let Err(e) = self.send_message_internal(&msg).await {
                warn!(error = %e, "Failed to send unsubscribe");
            }
        }
    }

    /// Handle a request command.
    async fn handle_request(
        &mut self,
        message: WsMessage,
        request_id: RequestId,
        reply_tx: oneshot::Sender<TransportResult<String>>,
        timeout: Option<Duration>,
    ) {
        // Add to pending requests first
        let rx = match self.pending_requests.add(request_id.clone(), timeout) {
            Some(rx) => rx,
            None => {
                let _ = reply_tx.send(Err(TransportError::capacity_exceeded(
                    "Too many pending requests",
                )));
                return;
            }
        };

        // Inject request ID into message
        let message = self.handler.inject_request_id(message, &request_id);

        // Send the message
        if let Err(e) = self.send_message_internal(&message).await {
            // Remove from pending and notify
            let err_msg = e.to_string();
            self.pending_requests
                .resolve(&request_id, Err(TransportError::websocket(&err_msg)));
            let _ = reply_tx.send(Err(e));
            return;
        }

        // Spawn a task to forward the response
        tokio::spawn(async move {
            match rx.await {
                Ok(result) => {
                    let _ = reply_tx.send(result);
                }
                Err(_) => {
                    let _ = reply_tx.send(Err(TransportError::internal("Request channel dropped")));
                }
            }
        });
    }

    /// Send a ping message.
    async fn send_ping(&mut self) -> TransportResult<()> {
        if self.config.use_websocket_ping {
            // WebSocket protocol-level ping
            if let Some(ref mut ws) = self.ws_write {
                ws.send(Message::Ping(Bytes::new()))
                    .await
                    .map_err(|e| TransportError::websocket(e.to_string()))?;
            }
        } else if let Some(ping) = self.handler.build_ping() {
            // Application-level ping
            self.send_message_internal(&ping).await?;
        }
        Ok(())
    }

    /// Send a message on the WebSocket.
    async fn send_message_internal(&mut self, msg: &WsMessage) -> TransportResult<()> {
        let ws = self
            .ws_write
            .as_mut()
            .ok_or_else(|| TransportError::websocket("Not connected"))?;

        let message = match msg {
            WsMessage::Text(text) => Message::text(text),
            WsMessage::Binary(data) => Message::binary(data.clone()),
        };

        ws.send(message)
            .await
            .map_err(|e| TransportError::websocket(e.to_string()))?;
        Ok(())
    }
}
