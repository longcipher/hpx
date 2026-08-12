//! High-load hotpath profiling harness for the hpx HTTP client and yawc
//! WebSocket backend.
//!
//! Starts a local HTTP + WebSocket echo server, then drives:
//! - 8 concurrent HTTP workers × 2,500 GET requests against `/users/{id}`
//!   (with and without a 404 endpoint), measuring end-to-end latency;
//! - a single WebSocket connection running 5,000 sequential echo round trips.
//!
//! Latency percentiles (p50/p90/p99/p999) are printed for both paths, and the
//! hotpath guard dumps the full profiling report (functions + futures + debug
//! gauges, with per-function allocations when `hotpath-alloc` is enabled) when
//! the process exits.
//!
//! Run with:
//!
//! ```text
//! cargo run --release -p hpx --example hpx_hotpath_load --features 'hotpath,hotpath-alloc,ws'
//! ```

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::unwrap_used,
    reason = "example binary prints its benchmark report and exercises error paths"
)]

use std::time::{Duration, Instant};

use axum::{
    Router,
    extract::{
        Path,
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use futures_util::StreamExt;
use hpx::{Client, hotpath::HotpathLayer, ws::message::Message};
use tokio::net::TcpListener;

const HTTP_TASKS: usize = 8;
const HTTP_PER_TASK: usize = 2_500;
const WS_ROUND_TRIPS: usize = 5_000;

async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(echo_ws)
}

async fn echo_ws(mut ws: WebSocket) {
    while let Some(Ok(msg)) = ws.recv().await {
        match msg {
            WsMessage::Text(text) => {
                let _ = ws.send(WsMessage::Text(text)).await;
            }
            WsMessage::Binary(data) => {
                let _ = ws.send(WsMessage::Binary(data)).await;
            }
            _ => {}
        }
    }
}

async fn start_server() -> (String, String) {
    let app = Router::new()
        .route(
            "/users/{id}",
            get(|Path(id): Path<u32>| async move {
                format!(r#"{{"id":{id},"name":"user{id}","active":true}}"#)
            }),
        )
        .route(
            "/missing",
            get(|| async { (axum::http::StatusCode::NOT_FOUND, "nope") }),
        )
        .route("/ws", get(ws_handler));

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (addr.to_string(), format!("ws://{addr}/ws"))
}

fn print_percentiles(name: &str, mut samples: Vec<Duration>) {
    samples.sort_unstable();
    let len = samples.len();
    let at = |p: f64| samples[(p * (len as f64 - 1.0)) as usize].as_secs_f64() * 1e3;
    let total: u64 = samples.iter().map(|d| d.as_nanos() as u64).sum();
    let avg = total as f64 / len as f64 / 1e6;
    println!(
        "[{name}] n={len} avg={avg:.3}ms p50={:.3}ms p90={:.3}ms p99={:.3}ms p999={:.3}ms",
        at(0.5),
        at(0.9),
        at(0.99),
        at(0.999),
    );
}

fn print_hpx_report() {
    let stats = hpx::hotpath::snapshot();
    if stats.is_empty() {
        return;
    }
    println!("[hpx hotpath] per-endpoint HTTP report:");
    for stat in stats {
        let statuses = stat
            .statuses
            .iter()
            .map(|(s, c)| format!("{s}x{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  {:<44} count={:<4} errors={:<3} total={:?} avg={:?} statuses=[{statuses}]",
            stat.endpoint,
            stat.count,
            stat.error_count,
            stat.total,
            stat.avg(),
        );
    }
}

#[tokio::main]
#[hotpath::main(percentiles = [50.0, 90.0, 99.0, 99.9], limit = 40)]
async fn main() -> Result<(), hpx::Error> {
    let (http_addr, ws_url) = start_server().await;
    // Set NO_LAYER=1 to bypass HotpathLayer and measure the plain typed client
    // stack (also avoids forcing boxed dispatch).
    let mut builder = Client::builder();
    if std::env::var("NO_LAYER").is_err() {
        builder = builder.layer(HotpathLayer::new());
    }
    let client = std::sync::Arc::new(builder.build()?);

    // ---- HTTP load ----
    let http_start = Instant::now();
    let samples = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Duration>::new()));
    let mut handles = Vec::new();
    for task in 0..HTTP_TASKS {
        let client = std::sync::Arc::clone(&client);
        let samples = std::sync::Arc::clone(&samples);
        let base = http_addr.clone();
        handles.push(tokio::spawn(async move {
            let mut local = Vec::with_capacity(HTTP_PER_TASK);
            for i in 0..HTTP_PER_TASK {
                let id = (task * HTTP_PER_TASK + i) % 100;
                let start = Instant::now();
                let resp = client
                    .get(format!("http://{base}/users/{id}"))
                    .send()
                    .await
                    .expect("send");
                let _body = resp.bytes().await.expect("body");
                local.push(start.elapsed());
            }
            samples.lock().expect("lock").extend(local);
        }));
    }
    for handle in handles {
        handle.await.expect("join");
    }
    let http_total = http_start.elapsed();
    let http_samples = std::sync::Arc::try_unwrap(samples)
        .expect("samples")
        .into_inner()
        .expect("samples");
    println!(
        "[http] {} requests in {http_total:?} ({:.0} req/s)",
        HTTP_TASKS * HTTP_PER_TASK,
        (HTTP_TASKS * HTTP_PER_TASK) as f64 / http_total.as_secs_f64()
    );
    print_percentiles("http", http_samples);

    // One 404 to exercise the error-counting path.
    let resp = client
        .get(format!("http://{http_addr}/missing"))
        .send()
        .await?;
    let _ = resp.bytes().await?;

    // ---- WebSocket load ----
    let ws_resp = hpx::websocket(&ws_url).send().await?;
    let ws = ws_resp.into_websocket().await?;
    let (mut tx, mut rx) = ws.split();

    let ws_start = Instant::now();
    let mut ws_samples = Vec::with_capacity(WS_ROUND_TRIPS);
    for _ in 0..WS_ROUND_TRIPS {
        let start = Instant::now();
        tx.send(Message::text("ping")).await?;
        let echo = rx.next().await.expect("echo").expect("echo ok");
        let _len = echo.len();
        ws_samples.push(start.elapsed());
    }
    let ws_total = ws_start.elapsed();
    println!(
        "[ws] {WS_ROUND_TRIPS} echo round trips in {ws_total:?} ({:.0} msg/s)",
        WS_ROUND_TRIPS as f64 / ws_total.as_secs_f64()
    );
    print_percentiles("ws", ws_samples);
    tx.close(hpx::ws::message::CloseCode::NORMAL, "done")
        .await?;

    print_hpx_report();
    println!("hpx_hotpath_load completed");
    Ok(())
}
