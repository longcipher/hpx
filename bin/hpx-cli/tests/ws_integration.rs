#![allow(clippy::unwrap_used, clippy::panic, clippy::match_wild_err_arm)]

//! Integration tests for WebSocket CLI commands using a mock axum server.

use axum::{
    Router,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
};
use tokio::net::TcpListener;

async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(echo_ws)
}

async fn echo_ws(mut ws: WebSocket) {
    while let Some(Ok(msg)) = ws.recv().await {
        match msg {
            Message::Text(text) => {
                let _ = ws.send(Message::Text(text)).await;
                return;
            }
            Message::Binary(data) => {
                let _ = ws.send(Message::Binary(data)).await;
                return;
            }
            _ => {}
        }
    }
}

async fn start_ws_server() -> String {
    let app = Router::new().route("/ws", get(ws_handler));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("{addr}")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_ws_echo() {
    let addr = start_ws_server().await;

    let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_hpx"))
        .args(["--data", "hello", &format!("ws://{addr}/ws")])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    let result =
        tokio::time::timeout(std::time::Duration::from_secs(5), child.wait_with_output()).await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                stdout.contains("hello"),
                "expected 'hello' in stdout, got: {stdout}"
            );
        }
        Ok(Err(e)) => {
            panic!("failed to wait for process: {e}");
        }
        Err(_timeout_err) => {
            panic!("test timed out");
        }
    }
}
