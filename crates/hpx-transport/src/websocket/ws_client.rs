//! User-facing WebSocket client API.
//!
//! The [`WsClient`] provides a clean, ergonomic interface for WebSocket
//! communication with support for both request-response and subscription patterns.

use std::{marker::PhantomData, sync::Arc, time::Duration};

use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::info;

use super::{
    actor::{ActorCommand, ConnectionActor},
    config::WsConfig,
    pending::PendingRequestStore,
    protocol::{ProtocolHandler, WsMessage},
    subscription::SubscriptionStore,
    types::{RequestId, Topic},
};
use crate::error::{TransportError, TransportResult};

/// WebSocket client for dual-pattern communication.
///
/// Supports both:
/// - **Request-Response**: Correlated requests with automatic timeout
/// - **Subscriptions**: Topic-based event streams via broadcast channels
///
/// The client is cheap to clone and can be shared across tasks.
///
/// # Example
///
/// ```rust,ignore
/// let config = WsConfig::new("wss://api.example.com/ws");
/// let handler = MyProtocolHandler::new();
/// let client = WsClient::connect(config, handler).await?;
///
/// // Subscribe to a topic
/// let mut rx = client.subscribe("orderbook.BTC").await?;
///
/// // Send a request
/// let response: MyResponse = client.request(&MyRequest::new()).await?;
///
/// // Receive subscription updates
/// while let Ok(msg) = rx.recv().await {
///     println!("Update: {:?}", msg);
/// }
/// ```
#[derive(Clone)]
pub struct WsClient<H: ProtocolHandler> {
    /// Channel to send commands to the actor.
    cmd_tx: mpsc::Sender<ActorCommand>,
    /// Shared pending request store.
    pending_requests: Arc<PendingRequestStore>,
    /// Shared subscription store.
    subscriptions: Arc<SubscriptionStore>,
    /// Configuration (for reading settings).
    config: Arc<WsConfig>,
    /// Protocol handler marker.
    _marker: PhantomData<H>,
}

impl<H: ProtocolHandler> WsClient<H> {
    /// Connect to a WebSocket server.
    ///
    /// Spawns a background actor to manage the connection lifecycle.
    pub async fn connect(config: WsConfig, handler: H) -> TransportResult<Self> {
        // Validate configuration
        config.validate().map_err(TransportError::config)?;

        let config = Arc::new(config);
        let (cmd_tx, cmd_rx) = mpsc::channel(config.command_channel_capacity);
        let pending_requests = Arc::new(PendingRequestStore::new(Arc::clone(&config)));
        let subscriptions = Arc::new(SubscriptionStore::new(Arc::clone(&config)));

        // Create and spawn the actor
        let actor = ConnectionActor::new(
            Arc::clone(&config),
            handler,
            cmd_rx,
            Arc::clone(&pending_requests),
            Arc::clone(&subscriptions),
        );

        tokio::spawn(actor.run());

        info!(url = %config.url, "WebSocket client created");

        Ok(Self {
            cmd_tx,
            pending_requests,
            subscriptions,
            config,
            _marker: PhantomData,
        })
    }

    /// Check if the client can send commands.
    ///
    /// Returns `false` if the actor has shut down.
    pub fn is_connected(&self) -> bool {
        !self.cmd_tx.is_closed()
    }

    // ========================================================================
    // Request-Response API
    // ========================================================================

    /// Send a typed request and await a typed response.
    ///
    /// Uses the default timeout from configuration.
    pub async fn request<R, T>(&self, request: &R) -> TransportResult<T>
    where
        R: Serialize,
        T: DeserializeOwned,
    {
        self.request_with_timeout(request, None).await
    }

    /// Send a typed request with a custom timeout.
    pub async fn request_with_timeout<R, T>(
        &self,
        request: &R,
        timeout: Option<Duration>,
    ) -> TransportResult<T>
    where
        R: Serialize,
        T: DeserializeOwned,
    {
        let json = serde_json::to_string(request)?;
        let response = self
            .request_raw_with_timeout(WsMessage::text(json), timeout)
            .await?;
        let parsed: T = serde_json::from_str(&response)?;
        Ok(parsed)
    }

    /// Send a raw message and await a raw response.
    pub async fn request_raw(&self, message: WsMessage) -> TransportResult<String> {
        self.request_raw_with_timeout(message, None).await
    }

    /// Send a raw message with a custom timeout.
    pub async fn request_raw_with_timeout(
        &self,
        message: WsMessage,
        timeout: Option<Duration>,
    ) -> TransportResult<String> {
        // Check capacity
        if !self.pending_requests.has_capacity() {
            return Err(TransportError::capacity_exceeded(
                "Too many pending requests",
            ));
        }

        let request_id = RequestId::new();
        let (reply_tx, reply_rx) = oneshot::channel();

        // Send command to actor
        self.cmd_tx
            .send(ActorCommand::Request {
                message,
                request_id: request_id.clone(),
                reply_tx,
                timeout,
            })
            .await
            .map_err(|_| TransportError::connection_closed(Some("Actor shut down".to_string())))?;

        // Wait for response
        let effective_timeout = timeout.unwrap_or(self.config.request_timeout);
        match tokio::time::timeout(effective_timeout, reply_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(TransportError::internal("Response channel dropped")),
            Err(_) => Err(TransportError::request_timeout(
                effective_timeout,
                request_id.to_string(),
            )),
        }
    }

    // ========================================================================
    // Subscription API
    // ========================================================================

    /// Subscribe to a topic.
    ///
    /// Returns a broadcast receiver for subscription updates.
    pub async fn subscribe(
        &self,
        topic: impl Into<Topic>,
    ) -> TransportResult<broadcast::Receiver<WsMessage>> {
        let topic = topic.into();

        // Register in local store first
        let (rx, is_new) = self.subscriptions.subscribe(topic.clone());

        // If new, tell the actor to send subscribe message
        if is_new {
            let (reply_tx, reply_rx) = oneshot::channel();

            self.cmd_tx
                .send(ActorCommand::Subscribe {
                    topics: vec![topic],
                    reply_tx,
                })
                .await
                .map_err(|_| {
                    TransportError::connection_closed(Some("Actor shut down".to_string()))
                })?;

            // Wait for confirmation
            match reply_rx.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err(TransportError::internal("Subscribe channel dropped")),
            }
        }

        Ok(rx)
    }

    /// Subscribe to multiple topics.
    pub async fn subscribe_many(
        &self,
        topics: impl IntoIterator<Item = impl Into<Topic>>,
    ) -> TransportResult<Vec<broadcast::Receiver<WsMessage>>> {
        let topics: Vec<Topic> = topics.into_iter().map(Into::into).collect();
        let mut receivers = Vec::with_capacity(topics.len());
        let mut new_topics = Vec::new();

        for topic in &topics {
            let (rx, is_new) = self.subscriptions.subscribe(topic.clone());
            receivers.push(rx);
            if is_new {
                new_topics.push(topic.clone());
            }
        }

        // Send subscribe for new topics
        if !new_topics.is_empty() {
            let (reply_tx, reply_rx) = oneshot::channel();

            self.cmd_tx
                .send(ActorCommand::Subscribe {
                    topics: new_topics,
                    reply_tx,
                })
                .await
                .map_err(|_| {
                    TransportError::connection_closed(Some("Actor shut down".to_string()))
                })?;

            match reply_rx.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err(TransportError::internal("Subscribe channel dropped")),
            }
        }

        Ok(receivers)
    }

    /// Unsubscribe from a topic.
    pub async fn unsubscribe(&self, topic: impl Into<Topic>) -> TransportResult<()> {
        let topic = topic.into();

        // Decrement ref count locally
        let was_last = self.subscriptions.unsubscribe(&topic);

        // If last subscriber, tell actor to unsubscribe
        if was_last {
            self.cmd_tx
                .send(ActorCommand::Unsubscribe {
                    topics: vec![topic],
                })
                .await
                .map_err(|_| {
                    TransportError::connection_closed(Some("Actor shut down".to_string()))
                })?;
        }

        Ok(())
    }

    // ========================================================================
    // Low-Level API
    // ========================================================================

    /// Send a message without expecting a response.
    pub async fn send(&self, message: WsMessage) -> TransportResult<()> {
        self.cmd_tx
            .send(ActorCommand::Send { message })
            .await
            .map_err(|_| TransportError::connection_closed(Some("Actor shut down".to_string())))
    }

    /// Send a JSON message without expecting a response.
    pub async fn send_json<T: Serialize>(&self, payload: &T) -> TransportResult<()> {
        let json = serde_json::to_string(payload)?;
        self.send(WsMessage::text(json)).await
    }

    /// Close the connection gracefully.
    pub async fn close(&self) -> TransportResult<()> {
        let _ = self.cmd_tx.send(ActorCommand::Close).await;
        Ok(())
    }

    /// Get the number of pending requests.
    pub fn pending_count(&self) -> usize {
        self.pending_requests.len()
    }

    /// Get the number of active subscriptions.
    pub fn subscription_count(&self) -> usize {
        self.subscriptions.len()
    }

    /// Get all subscribed topics.
    pub fn subscribed_topics(&self) -> Vec<Topic> {
        self.subscriptions.get_all_topics()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal protocol handler for testing.
    struct TestHandler;

    impl ProtocolHandler for TestHandler {
        fn classify_message(&self, _message: &str) -> super::super::types::MessageKind {
            super::super::types::MessageKind::Update
        }

        fn extract_topic(&self, _message: &str) -> Option<Topic> {
            None
        }

        fn extract_request_id(&self, _message: &str) -> Option<RequestId> {
            None
        }

        fn build_subscribe(&self, _topics: &[Topic], _request_id: RequestId) -> WsMessage {
            WsMessage::text("{}")
        }

        fn build_unsubscribe(&self, _topics: &[Topic], _request_id: RequestId) -> WsMessage {
            WsMessage::text("{}")
        }
    }

    #[test]
    fn test_ws_client_is_send_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}

        // Verify WsClient is Send + Sync (required for sharing across tasks)
        assert_send::<WsClient<TestHandler>>();
        assert_sync::<WsClient<TestHandler>>();
    }
}
