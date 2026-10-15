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
//!
//! # Two-process mode
//!
//! By default the server and the load generator share one process and one tokio
//! runtime, so the client's CPU is indistinguishable from the server's and the
//! measured throughput swings by ±20% run to run — far too noisy to resolve
//! client-side optimizations. Splitting them into two processes separates the
//! runtimes and makes client-side changes measurable:
//!
//! ```text
//! # terminal 1
//! HPX_LOAD_MODE=server HPX_LOAD_PORT=60123 cargo run --release ... &
//! # terminal 2
//! HPX_LOAD_MODE=client HPX_LOAD_HTTP_ADDR=127.0.0.1:60123 \
//!   HPX_LOAD_WS_URL=ws://127.0.0.1:60123/ws cargo run --release ...
//! ```
//!
//! Workload size is also overridable (`HPX_HTTP_TASKS`, `HPX_HTTP_PER_TASK`,
//! `HPX_WS_ROUND_TRIPS`); the defaults run for only ~0.35 s, which is too short
//! to average out scheduling noise. Raising `HPX_HTTP_PER_TASK` to ~25k makes
//! each run several seconds long.

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::unwrap_used,
    reason = "example binary prints its benchmark report and exercises error paths"
)]
#![allow(
    clippy::allow_attributes,
    clippy::assert_is_empty,
    clippy::borrow_as_ptr,
    clippy::cast_lossless,
    clippy::collapsible_if,
    clippy::collection_is_never_read,
    clippy::duration_suboptimal_units,
    clippy::expect_fun_call,
    clippy::expect_used,
    clippy::explicit_iter_loop,
    clippy::ignore_without_reason,
    clippy::ignored_unit_patterns,
    clippy::items_after_test_module,
    clippy::iter_kv_map,
    clippy::iter_with_drain,
    clippy::let_underscore_future,
    clippy::manual_div_ceil,
    clippy::manual_string_new,
    clippy::map_unwrap_or,
    clippy::needless_borrows_for_generic_args,
    clippy::needless_continue,
    clippy::needless_pass_by_ref_mut,
    clippy::needless_raw_string_hashes,
    clippy::nonminimal_bool,
    clippy::panic,
    clippy::redundant_async_block,
    clippy::redundant_clone,
    clippy::redundant_pattern_matching,
    clippy::struct_field_names,
    clippy::type_complexity,
    clippy::unchecked_time_subtraction,
    clippy::unnecessary_cast,
    clippy::unnecessary_trailing_comma,
    clippy::unnested_or_patterns,
    clippy::unreadable_literal,
    clippy::use_self,
    clippy::useless_borrows_in_formatting,
    clippy::useless_vec,
    missing_docs,
    unfulfilled_lint_expectations
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

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
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

    // Port 0 lets the OS pick; `HPX_LOAD_PORT` pins it so a separate client
    // process can find the server (see the two-process notes in the module doc).
    let port = u16::try_from(env_usize("HPX_LOAD_PORT", 0)).expect("port in range");
    let listener = TcpListener::bind(("127.0.0.1", port)).await.expect("bind");
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

// `#[hotpath::main]` only exists with the `hotpath` feature. Gating it lets the
// same example run as a *production* benchmark (no per-function timers, no
// futures tracking) with `--features ws`, which is what you want when comparing
// builds for performance rather than looking for hotspots. It must stay below
// `#[tokio::main]`: attribute macros apply bottom-up, so `hotpath::main` has to
// see the `async fn` before `tokio::main` rewrites it.
#[tokio::main]
#[cfg_attr(
    feature = "hotpath",
    hotpath::main(percentiles = [50.0, 90.0, 99.0, 99.9], limit = 40)
)]
async fn main() -> Result<(), hpx::Error> {
    let (http_addr, ws_url) = match std::env::var("HPX_LOAD_MODE").unwrap_or_default().as_str() {
        // Serve only, and stay up until the client process is done with us.
        "server" => {
            let (http_addr, ws_url) = start_server().await;
            println!("[mode=server] http_addr={http_addr} ws_url={ws_url}");
            std::future::pending::<()>().await;
            return Ok(());
        }
        // Drive load only, against a server started with `HPX_LOAD_MODE=server`.
        "client" => (
            std::env::var("HPX_LOAD_HTTP_ADDR").expect("HPX_LOAD_HTTP_ADDR"),
            std::env::var("HPX_LOAD_WS_URL").expect("HPX_LOAD_WS_URL"),
        ),
        _ => start_server().await,
    };

    let http_tasks = env_usize("HPX_HTTP_TASKS", HTTP_TASKS);
    let http_per_task = env_usize("HPX_HTTP_PER_TASK", HTTP_PER_TASK);
    let ws_round_trips = env_usize("HPX_WS_ROUND_TRIPS", WS_ROUND_TRIPS);

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
    for task in 0..http_tasks {
        let client = std::sync::Arc::clone(&client);
        let samples = std::sync::Arc::clone(&samples);
        let base = http_addr.clone();
        handles.push(tokio::spawn(async move {
            let mut local = Vec::with_capacity(http_per_task);
            for i in 0..http_per_task {
                let id = (task * http_per_task + i) % 100;
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
        http_tasks * http_per_task,
        (http_tasks * http_per_task) as f64 / http_total.as_secs_f64()
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
    let mut ws_samples = Vec::with_capacity(ws_round_trips);
    for _ in 0..ws_round_trips {
        let start = Instant::now();
        tx.send(Message::text("ping")).await?;
        let echo = rx.next().await.expect("echo").expect("echo ok");
        let _len = echo.len();
        ws_samples.push(start.elapsed());
    }
    let ws_total = ws_start.elapsed();
    println!(
        "[ws] {ws_round_trips} echo round trips in {ws_total:?} ({:.0} msg/s)",
        ws_round_trips as f64 / ws_total.as_secs_f64()
    );
    print_percentiles("ws", ws_samples);
    tx.close(hpx::ws::message::CloseCode::NORMAL, "done")
        .await?;

    print_hpx_report();
    println!("hpx_hotpath_load completed");
    Ok(())
}
