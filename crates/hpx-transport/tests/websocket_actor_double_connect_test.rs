use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use futures::{SinkExt, StreamExt};
use hpx_transport::{
    error::TransportResult,
    websocket::{MessageKind, ProtocolHandler, RequestId, Topic, WsClient, WsConfig, WsMessage},
};
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
use tokio::{net::TcpListener, sync::oneshot};

struct ServerState {
    conn_count: AtomicU64,
    second_conn_tx: Mutex<Option<oneshot::Sender<()>>>,
}

async fn start_test_server(state: Arc<ServerState>) -> std::io::Result<String> {
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
                let conn_id = state.conn_count.fetch_add(1, Ordering::Relaxed) + 1;
                if conn_id == 2 {
                    if let Some(tx) = state.second_conn_tx.lock().unwrap().take() {
                        let _ = tx.send(());
                    }
                }

                let io = TokioIo::new(stream);
                let conn_fut = http1::Builder::new()
                    .serve_connection(
                        io,
                        service_fn(move |req| server_upgrade(req, Arc::clone(&state))),
                    )
                    .with_upgrades();
                let _ = conn_fut.await;
            });
        }
    });

    Ok(format!("ws://{}", addr))
}

async fn server_upgrade(
    mut req: Request<Incoming>,
    state: Arc<ServerState>,
) -> hpx_yawc::Result<Response<Empty<Bytes>>> {
    let (response, fut) = WebSocket::upgrade_with_options(&mut req, Options::default())?;

    tokio::spawn(async move {
        let mut ws = match fut.await {
            Ok(ws) => ws,
            Err(_) => return,
        };

        let mut authed = false;
        let mut update_sent = false;

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
                authed = true;
                continue;
            }

            if text.starts_with("SUB:") && authed && !update_sent {
                update_sent = true;
                let _ = ws.send(Frame::text("UPDATE:trades.BTC:1")).await;
            }
        }

        drop(state);
    });

    Ok(response)
}

#[derive(Clone)]
struct TestHandler;

impl ProtocolHandler for TestHandler {
    fn on_connect(&self) -> Vec<WsMessage> {
        Vec::new()
    }

    fn build_auth_message(&self) -> Option<WsMessage> {
        Some(WsMessage::text("AUTH"))
    }

    fn is_auth_success(&self, message: &str) -> bool {
        message == "AUTH_OK"
    }

    fn is_auth_failure(&self, message: &str) -> bool {
        message == "AUTH_FAIL"
    }

    fn classify_message(&self, message: &str) -> MessageKind {
        if message.starts_with("UPDATE:") {
            MessageKind::Update
        } else {
            MessageKind::Control
        }
    }

    fn extract_topic(&self, message: &str) -> Option<Topic> {
        let mut parts = message.splitn(3, ':');
        let tag = parts.next()?;
        let topic = parts.next()?;
        if tag == "UPDATE" {
            Some(Topic::from(topic))
        } else {
            None
        }
    }

    fn extract_request_id(&self, _message: &str) -> Option<RequestId> {
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

#[tokio::test]
async fn test_actor_reuses_authenticated_connection() -> TransportResult<()> {
    let (second_tx, mut second_rx) = oneshot::channel();
    let state = Arc::new(ServerState {
        conn_count: AtomicU64::new(0),
        second_conn_tx: Mutex::new(Some(second_tx)),
    });

    let url = start_test_server(Arc::clone(&state)).await.expect("server");
    let config = WsConfig::new(url)
        .auth_on_connect(true)
        .ping_interval(std::time::Duration::from_secs(60))
        .request_timeout(std::time::Duration::from_secs(2));

    let client = WsClient::connect(config, TestHandler).await?;
    let mut sub_guard = client.subscribe("trades.BTC").await?;

    let update = tokio::select! {
        _ = &mut second_rx => {
            return Err(hpx_transport::error::TransportError::internal(
                "observed unexpected second connection",
            ));
        }
        update = sub_guard.recv() => update.ok_or_else(|| {
            hpx_transport::error::TransportError::internal("subscription channel closed")
        })?,
        _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {
            return Err(hpx_transport::error::TransportError::internal(
                "timed out waiting for update",
            ));
        }
    };

    match update {
        WsMessage::Text(text) if text == "UPDATE:trades.BTC:1" => Ok(()),
        WsMessage::Text(text) => Err(hpx_transport::error::TransportError::internal(format!(
            "unexpected update payload: {text}",
        ))),
        _ => Err(hpx_transport::error::TransportError::internal(
            "unexpected non-text update",
        )),
    }
}
