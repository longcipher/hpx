//! User-facing WebSocket client API.
//!
//! The [`WsClient`] provides a clean, ergonomic interface for WebSocket
//! communication with support for both request-response and subscription patterns.

use std::{marker::PhantomData, sync::Arc, time::Duration};

use serde::{Serialize, de::DeserializeOwned};
use tokio::{sync::broadcast, task::JoinHandle};
use tracing::info;

use super::{
    config::WsConfig,
    connection::Connection,
    protocol::{ProtocolHandler, WsMessage},
    types::Topic,
};
use crate::error::TransportResult;

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
    /// Handle to the underlying connection.
    handle: super::connection::ConnectionHandle,
    /// Background task draining the event stream.
    _stream_task: Arc<JoinHandle<()>>,
    /// Protocol handler marker.
    _marker: PhantomData<H>,
}

impl<H: ProtocolHandler> WsClient<H> {
    /// Connect to a WebSocket server.
    ///
    /// Spawns a background actor to manage the connection lifecycle.
    pub async fn connect(config: WsConfig, handler: H) -> TransportResult<Self> {
        let url = config.url.clone();
        let (handle, stream) = Connection::connect(config, handler).await?;
        let stream_task = tokio::spawn(async move {
            let mut stream = stream;
            while let Some(_event) = stream.next().await {}
        });

        info!(url = %url, "WebSocket client created");

        Ok(Self {
            handle,
            _stream_task: Arc::new(stream_task),
            _marker: PhantomData,
        })
    }

    /// Check if the client can send commands.
    ///
    /// Returns `false` if the actor has shut down.
    pub fn is_connected(&self) -> bool {
        self.handle.is_connected()
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
        self.handle.request_with_timeout(request, timeout).await
    }

    /// Send a raw message and await a raw response.
    pub async fn request_raw(&self, message: WsMessage) -> TransportResult<String> {
        self.handle.request_raw(message).await
    }

    /// Send a raw message with a custom timeout.
    pub async fn request_raw_with_timeout(
        &self,
        message: WsMessage,
        timeout: Option<Duration>,
    ) -> TransportResult<String> {
        self.handle.request_raw_with_timeout(message, timeout).await
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
        let guard = self.handle.subscribe(topic).await?;
        Ok(guard.into_receiver())
    }

    /// Subscribe to multiple topics.
    pub async fn subscribe_many(
        &self,
        topics: impl IntoIterator<Item = impl Into<Topic>>,
    ) -> TransportResult<Vec<broadcast::Receiver<WsMessage>>> {
        let guards = self.handle.subscribe_many(topics).await?;
        Ok(guards
            .into_iter()
            .map(|guard| guard.into_receiver())
            .collect())
    }

    /// Unsubscribe from a topic.
    pub async fn unsubscribe(&self, topic: impl Into<Topic>) -> TransportResult<()> {
        self.handle.unsubscribe(topic).await
    }

    // ========================================================================
    // Low-Level API
    // ========================================================================

    /// Send a message without expecting a response.
    pub async fn send(&self, message: WsMessage) -> TransportResult<()> {
        self.handle.send(message).await
    }

    /// Send a JSON message without expecting a response.
    pub async fn send_json<T: Serialize>(&self, payload: &T) -> TransportResult<()> {
        let json = serde_json::to_string(payload)?;
        self.send(WsMessage::text(json)).await
    }

    /// Close the connection gracefully.
    pub async fn close(&self) -> TransportResult<()> {
        self.handle.close().await
    }

    /// Get the number of pending requests.
    pub fn pending_count(&self) -> usize {
        self.handle.pending_count()
    }

    /// Get the number of active subscriptions.
    pub fn subscription_count(&self) -> usize {
        self.handle.subscription_count()
    }

    /// Get all subscribed topics.
    pub fn subscribed_topics(&self) -> Vec<Topic> {
        self.handle.subscribed_topics()
    }
}

#[cfg(test)]
mod tests {
    use super::{super::types::RequestId, *};

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

    #[test]
    fn ws_client_exposes_connection_handle() {
        fn assert_has_handle<H: ProtocolHandler>(client: &WsClient<H>) {
            let _ = &client.handle;
        }

        let _ = assert_has_handle::<TestHandler>;
    }
}
