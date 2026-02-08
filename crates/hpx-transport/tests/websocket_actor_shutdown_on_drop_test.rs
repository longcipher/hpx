use std::sync::{Arc, Mutex};

use futures::StreamExt;
use hpx_transport::{
    error::TransportResult,
    websocket::{MessageKind, ProtocolHandler, RequestId, Topic, WsClient, WsConfig, WsMessage},
};
use hpx_yawc::{Options, WebSocket};
use http_body_util::Empty;
use hyper::{
    Request, Response,
    body::{Bytes, Incoming},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use tokio::{net::TcpListener, sync::oneshot, time::timeout};

struct ServerState {
    connected_tx: Mutex<Option<oneshot::Sender<()>>>,
    closed_tx: Mutex<Option<oneshot::Sender<()>>>,
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

        if let Some(tx) = state.connected_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }

        while let Some(_frame) = ws.next().await {
            // keep draining until disconnect
        }

        if let Some(tx) = state.closed_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    });

    Ok(response)
}

#[derive(Clone)]
struct TestHandler;

impl ProtocolHandler for TestHandler {
    fn classify_message(&self, _message: &str) -> MessageKind {
        MessageKind::Unknown
    }

    fn extract_topic(&self, _message: &str) -> Option<Topic> {
        None
    }

    fn extract_request_id(&self, _message: &str) -> Option<RequestId> {
        None
    }

    fn build_subscribe(&self, _topics: &[Topic], _request_id: RequestId) -> WsMessage {
        WsMessage::text("sub")
    }

    fn build_unsubscribe(&self, _topics: &[Topic], _request_id: RequestId) -> WsMessage {
        WsMessage::text("unsub")
    }
}

#[tokio::test]
async fn test_actor_shuts_down_on_handle_drop() -> TransportResult<()> {
    let (connected_tx, connected_rx) = oneshot::channel();
    let (closed_tx, closed_rx) = oneshot::channel();
    let state = Arc::new(ServerState {
        connected_tx: Mutex::new(Some(connected_tx)),
        closed_tx: Mutex::new(Some(closed_tx)),
    });

    let url = start_test_server(Arc::clone(&state)).await.expect("server");
    let config = WsConfig::new(url).ping_interval(std::time::Duration::from_secs(60));
    let client = WsClient::connect(config, TestHandler).await?;
    let client_clone = client.clone();

    timeout(std::time::Duration::from_secs(1), connected_rx)
        .await
        .map_err(|_| hpx_transport::error::TransportError::internal("connect timeout"))?
        .map_err(|_| hpx_transport::error::TransportError::internal("connect channel closed"))?;

    drop(client);
    drop(client_clone);

    timeout(std::time::Duration::from_secs(1), closed_rx)
        .await
        .map_err(|_| hpx_transport::error::TransportError::internal("shutdown timeout"))?
        .map_err(|_| hpx_transport::error::TransportError::internal("shutdown channel closed"))?;

    Ok(())
}
