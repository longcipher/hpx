use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures_util::Stream;
use serde::{Serialize, de::DeserializeOwned};
use tokio::{
    sync::{broadcast, mpsc},
    time::timeout,
};

use super::{
    config::WsConfig,
    pending::PendingRequestStore,
    protocol::WsMessage,
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use tokio::sync::mpsc;

    use super::*;

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
}
