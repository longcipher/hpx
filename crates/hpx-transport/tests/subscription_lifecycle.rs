use std::{sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use hpx_transport::{
    error::{TransportError, TransportResult},
    websocket::{
        Connection, Event, MessageKind, ProtocolHandler, RequestId, Topic, WsConfig, WsMessage,
    },
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
use tokio::{net::TcpListener, sync::mpsc, time::timeout};

struct ServerState {
    msg_tx: mpsc::UnboundedSender<String>,
}

async fn start_test_server() -> std::io::Result<(String, mpsc::UnboundedReceiver<String>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (msg_tx, msg_rx) = mpsc::unbounded_channel();
    let state = Arc::new(ServerState { msg_tx });

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

    Ok((format!("ws://{}", addr), msg_rx))
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

        while let Some(frame) = ws.next().await {
            match frame.opcode() {
                OpCode::Text => {
                    if let Ok(text) = std::str::from_utf8(frame.payload()) {
                        let _ = state.msg_tx.send(text.to_string());
                    }
                }
                OpCode::Ping => {
                    let _ = ws.send(Frame::pong(frame.payload().clone())).await;
                }
                _ => {}
            }
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

async fn wait_for_connected(
    stream: &mut hpx_transport::websocket::ConnectionStream,
) -> TransportResult<()> {
    let event = timeout(Duration::from_secs(3), stream.next())
        .await
        .map_err(|_| TransportError::internal("timeout waiting for connected event"))?
        .ok_or_else(|| TransportError::internal("event stream closed"))?;

    match event {
        Event::Connected { .. } => Ok(()),
        other => Err(TransportError::internal(format!(
            "unexpected event while waiting for connected: {other:?}",
        ))),
    }
}

async fn expect_message(
    rx: &mut mpsc::UnboundedReceiver<String>,
    expected: &str,
) -> TransportResult<()> {
    match timeout(Duration::from_secs(3), rx.recv()).await {
        Ok(Some(msg)) if msg == expected => Ok(()),
        Ok(Some(msg)) => Err(TransportError::internal(format!(
            "expected {expected}, got {msg}",
        ))),
        Ok(None) => Err(TransportError::internal("message channel closed")),
        Err(_) => Err(TransportError::internal(format!(
            "timeout waiting for {expected}",
        ))),
    }
}

async fn assert_no_message(
    rx: &mut mpsc::UnboundedReceiver<String>,
    duration: Duration,
    context: &str,
) -> TransportResult<()> {
    match timeout(duration, rx.recv()).await {
        Ok(Some(msg)) => Err(TransportError::internal(format!(
            "unexpected message during {context}: {msg}",
        ))),
        Ok(None) => Err(TransportError::internal("message channel closed")),
        Err(_) => Ok(()),
    }
}

#[tokio::test]
async fn test_subscription_drop_sends_unsubscribe() -> TransportResult<()> {
    let (url, mut rx) = start_test_server().await.expect("server");
    let config = WsConfig::new(url)
        .ping_interval(Duration::from_secs(60))
        .pong_timeout(Duration::from_secs(60));

    let (handle, mut stream) = Connection::connect(config, TestHandler).await?;
    wait_for_connected(&mut stream).await?;

    let _event_drain = tokio::spawn(async move { while stream.next().await.is_some() {} });

    let guard = handle.subscribe("trades.BTC").await?;
    expect_message(&mut rx, "SUB:trades.BTC").await?;

    drop(guard);
    expect_message(&mut rx, "UNSUB:trades.BTC").await?;

    handle.close().await?;
    Ok(())
}

#[tokio::test]
async fn test_drop_first_guard_does_not_unsubscribe() -> TransportResult<()> {
    let (url, mut rx) = start_test_server().await.expect("server");
    let config = WsConfig::new(url)
        .ping_interval(Duration::from_secs(60))
        .pong_timeout(Duration::from_secs(60));

    let (handle, mut stream) = Connection::connect(config, TestHandler).await?;
    wait_for_connected(&mut stream).await?;

    let _event_drain = tokio::spawn(async move { while stream.next().await.is_some() {} });

    let guard_one = handle.subscribe("trades.BTC").await?;
    expect_message(&mut rx, "SUB:trades.BTC").await?;

    let guard_two = handle.subscribe("trades.BTC").await?;
    assert_no_message(&mut rx, Duration::from_millis(200), "second subscribe").await?;

    drop(guard_one);
    assert_no_message(&mut rx, Duration::from_millis(200), "first guard drop").await?;

    drop(guard_two);
    expect_message(&mut rx, "UNSUB:trades.BTC").await?;

    handle.close().await?;
    Ok(())
}

#[tokio::test]
async fn test_explicit_unsubscribe_suppresses_drop_unsubscribe() -> TransportResult<()> {
    let (url, mut rx) = start_test_server().await.expect("server");
    let config = WsConfig::new(url)
        .ping_interval(Duration::from_secs(60))
        .pong_timeout(Duration::from_secs(60));

    let (handle, mut stream) = Connection::connect(config, TestHandler).await?;
    wait_for_connected(&mut stream).await?;

    let _event_drain = tokio::spawn(async move { while stream.next().await.is_some() {} });

    let guard = handle.subscribe("trades.BTC").await?;
    expect_message(&mut rx, "SUB:trades.BTC").await?;

    handle.unsubscribe("trades.BTC").await?;
    expect_message(&mut rx, "UNSUB:trades.BTC").await?;

    drop(guard);
    assert_no_message(
        &mut rx,
        Duration::from_millis(200),
        "guard drop after unsubscribe",
    )
    .await?;

    handle.close().await?;
    Ok(())
}
