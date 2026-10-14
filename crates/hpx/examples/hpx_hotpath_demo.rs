//! Demonstrates hotpath profiling integration in the hpx client.
//!
//! Builds a local HTTP + WebSocket echo server, runs a mix of requests
//! (200s, a 404, and a connection-refused transport error) plus a short
//! WebSocket echo conversation through a client instrumented with
//! [`hpx::hotpath::HotpathLayer`], prints the hpx-side per-endpoint report,
//! and lets the hotpath guard dump its own report (functions/futures/debug
//! sections) on exit.
//!
//! Run with:
//!
//! ```text
//! cargo run -p hpx --example hpx_hotpath_demo --features hotpath
//! ```
//!
//! Without the `hotpath` feature the example still compiles and runs, but no
//! profiling data is collected.
#![allow(
    clippy::allow_attributes,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::unwrap_used
)]
#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "example binary prints its demo report"
)]
#![allow(
    clippy::assert_is_empty,
    clippy::borrow_as_ptr,
    clippy::cast_lossless,
    clippy::collapsible_if,
    clippy::collection_is_never_read,
    clippy::duration_suboptimal_units,
    clippy::expect_fun_call,
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
use std::net::TcpListener;

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
use tokio::net::TcpListener as TokioTcpListener;

/// Echoes back the first text/binary message it receives.
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
        .route("/ok", get(|| async { "ok" }))
        .route(
            "/users/{id}",
            get(|Path(id): Path<u32>| async move { format!("user {id}") }),
        )
        .route(
            "/missing",
            get(|| async { (axum::http::StatusCode::NOT_FOUND, "nope") }),
        )
        .route("/ws", get(ws_handler));

    let listener = TokioTcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (addr.to_string(), format!("ws://{addr}/ws"))
}

fn start_dropped_port() -> u16 {
    // Bind then drop: nothing listens here, so requests fail to connect.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.local_addr().expect("local addr").port()
}

fn print_hpx_report() {
    let stats = hpx::hotpath::snapshot();
    if stats.is_empty() {
        println!("[hpx hotpath] no endpoint stats recorded (enable the `hotpath` feature)");
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
            "  {:<44} count={:<3} errors={:<2} total={:?} avg={:?} statuses=[{statuses}]",
            stat.endpoint,
            stat.count,
            stat.error_count,
            stat.total,
            stat.avg(),
        );
    }
}

#[tokio::main]
async fn main() -> Result<(), hpx::Error> {
    // Guard dumps hotpath's own report (functions, futures, debug gauges, ...)
    // when it drops at the end of main.
    let _guard = hotpath::HotpathGuardBuilder::new("hpx_hotpath_demo").build();

    let (http_addr, ws_url) = start_server().await;

    let client = Client::builder().layer(HotpathLayer::new()).build()?;

    // Two ids collapse into one normalized endpoint bucket.
    for id in 1..=2 {
        let resp = client
            .get(format!("http://{http_addr}/users/{id}"))
            .send()
            .await?;
        let body = resp.text().await?;
        println!("GET /users/{id} -> {body}");
    }

    let resp = client.get(format!("http://{http_addr}/ok")).send().await?;
    let body = resp.text().await?;
    println!("GET /ok -> {body}");

    let resp = client
        .get(format!("http://{http_addr}/missing"))
        .send()
        .await?;
    println!("GET /missing -> status {}", resp.status());

    // Transport error (connection refused) counts as an error in the report.
    let dead = start_dropped_port();
    let result = client
        .get(format!("http://127.0.0.1:{dead}/dead"))
        .send()
        .await;
    println!(
        "GET /dead (refused) -> {}",
        if result.is_err() { "error" } else { "ok" }
    );

    // WebSocket echo conversation through the yawc backend.
    let ws_resp = hpx::websocket(&ws_url).send().await?;
    let ws = ws_resp.into_websocket().await?;
    let (mut tx, mut rx) = ws.split();
    for i in 1..=5 {
        let text = format!("ping {i}");
        tx.send(Message::text(text.clone())).await?;
        if let Some(Ok(Message::Text(echo))) = rx.next().await {
            println!("ws echo: {echo}");
        }
    }
    tx.close(hpx::ws::message::CloseCode::NORMAL, "done")
        .await?;

    print_hpx_report();
    println!("hpx_hotpath_demo completed");
    Ok(())
}
