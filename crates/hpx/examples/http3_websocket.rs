//! WebSocket-over-h3 example (RFC 9220 Extended CONNECT).
//!
//! This example demonstrates how to use the hpx HTTP/3 client to establish a
//! WebSocket connection over HTTP/3 using the Extended CONNECT method defined
//! in [RFC 9220].
//!
//! Because there is no well-known public HTTP/3 WebSocket endpoint, this
//! example is self-documenting rather than runnable. The code compiles and
//! shows the correct API for each step of the WebSocket-over-h3 lifecycle:
//! handshake, send, receive, and shutdown.
//!
//! To compile:
//!
//! ```text
//! cargo build -p hpx --features http3 --example http3_websocket
//! ```
//!
//! [RFC 9220]: https://www.rfc-editor.org/rfc/rfc9220.html
#![allow(
    clippy::allow_attributes,
    clippy::assert_is_empty,
    clippy::borrow_as_ptr,
    clippy::cast_lossless,
    clippy::collection_is_never_read,
    clippy::collapsible_if,
    clippy::duration_suboptimal_units,
    clippy::explicit_iter_loop,
    clippy::expect_fun_call,
    clippy::expect_used,
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
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::redundant_async_block,
    clippy::redundant_clone,
    clippy::redundant_pattern_matching,
    clippy::struct_field_names,
    clippy::type_complexity,
    clippy::unchecked_time_subtraction,
    clippy::unreadable_literal,
    clippy::unnecessary_cast,
    clippy::unnecessary_trailing_comma,
    clippy::unnested_or_patterns,
    clippy::useless_borrows_in_formatting,
    clippy::useless_vec,
    clippy::use_self,
    clippy::unwrap_used,
    missing_docs,
    unfulfilled_lint_expectations
)]
#[cfg(feature = "http3")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ── Step 1: Create an HTTP/3-only client ──────────────────────────
    let client = hpx::Client::builder().http3_only().build()?;

    // ── Step 2: Build an Extended CONNECT request ─────────────────────
    // Per RFC 9220 §4, the client sends a CONNECT request with the
    // `:protocol` pseudo-header set to "websocket". The request URI is
    // the WebSocket endpoint URL.
    let req = http::Request::builder()
        .method("CONNECT")
        .uri("https://example.com/ws")
        .header(":protocol", "websocket")
        .body(hpx::Body::from(""))?;

    // ── Step 3: Execute the handshake ─────────────────────────────────
    let mut response = client.execute(req.into()).await?;
    println!("WebSocket handshake status: {}", response.status());

    // ── Step 4: Extract the H3WebSocket from the response ─────────────
    // After a successful Extended CONNECT (200 OK), the h3 request stream
    // becomes a bidirectional WebSocket data channel. The client stores
    // the stream in the response extensions as an `H3WebSocket`.
    let mut ws = hpx::http3::H3WebSocket::from_response(response.inner())
        .expect("Extended CONNECT response must contain an H3WebSocket");

    // ── Step 5: Send WebSocket messages ───────────────────────────────
    // All client-to-server frames are automatically masked per RFC 6455 §5.3.

    // Send a text frame.
    ws.send_text("Hello from h3 WebSocket!").await?;

    // Send a binary frame.
    ws.send_binary(&[0x00, 0x01, 0x02, 0x03]).await?;

    // Send a ping frame (the server should respond with a pong).
    ws.send_ping(b"keepalive").await?;

    // ── Step 6: Receive WebSocket messages ────────────────────────────
    // `recv()` returns `Ok(Some(WsMessage))` when a frame is available,
    // `Ok(None)` when the stream is closed cleanly, or an `Err` on error.
    // Ping frames are automatically answered with pong per RFC 6455 §5.5.3.
    loop {
        match ws.recv().await? {
            Some(hpx::http3::WsMessage::Text(text)) => {
                println!("Received text: {text}");
            }
            Some(hpx::http3::WsMessage::Binary(data)) => {
                println!("Received binary: {data:?}");
            }
            Some(hpx::http3::WsMessage::Pong(data)) => {
                println!("Received pong: {data:?}");
            }
            Some(hpx::http3::WsMessage::Close { code, reason }) => {
                println!("Server closed: code={code:?}, reason={reason:?}");
                // Echo the close frame back to complete the handshake.
                ws.send_close(code, reason.as_deref()).await?;
                break;
            }
            Some(hpx::http3::WsMessage::Ping(_)) => {
                // Already auto-answered by recv(); shouldn't see this.
                continue;
            }
            None => {
                println!("Stream closed by server");
                break;
            }
        }
    }

    // ── Step 7: Clean shutdown ────────────────────────────────────────
    // `finish()` half-closes the send direction of the stream.
    ws.finish().await?;

    Ok(())
}

#[cfg(not(feature = "http3"))]
fn main() {
    eprintln!("This example requires the `http3` feature.");
    eprintln!("Run: cargo build -p hpx --features http3 --example http3_websocket");
}
