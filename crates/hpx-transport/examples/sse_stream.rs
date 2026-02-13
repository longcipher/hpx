//! SSE Stream Example
//!
//! Demonstrates basic Server-Sent Events (SSE) transport usage.
//!
//! Run with: `cargo run -p hpx-transport --features sse --example sse_stream`

use std::time::Duration;

use hpx_transport::sse::{SseConfig, SseMessageKind};

fn main() {
    let _config = SseConfig::new("https://api.exchange.com/v1/stream")
        .connect_timeout(Duration::from_secs(10))
        .reconnect_max_attempts(Some(5))
        .reconnect_initial_delay(Duration::from_secs(1))
        .reconnect_max_delay(Duration::from_secs(60))
        .auth_on_connect(true);

    println!("SSE Stream Example");
    println!("==================");
    println!();
    println!("This example demonstrates how to use the SSE transport.");
    println!();
    println!("Usage:");
    println!("  1. Create an SseConfig with the SSE endpoint URL");
    println!("  2. Connect with a protocol handler (GenericSseHandler or custom)");
    println!("  3. Split into SseHandle (control) and SseStream (events)");
    println!("  4. Consume events from the stream");
    println!();
    println!("Example code:");
    println!();
    println!(
        r#"
    use hpx_transport::sse::{{
        SseConfig, SseConnection, SseMessageKind,
        handlers::GenericSseHandler,
    }};

    let config = SseConfig::new("https://api.exchange.com/v1/stream")
        .connect_timeout(Duration::from_secs(10))
        .reconnect_max_attempts(Some(5));

    let handler = GenericSseHandler::new();
    let connection = SseConnection::connect(config, handler).await?;
    let (handle, mut stream) = connection.split();

    // Consume events
    while let Some(event) = stream.next_event().await {{
        match event.kind {{
            SseMessageKind::Data => {{
                println!(
                    "Event type: {{}}, data: {{}}",
                    event.event_type(),
                    event.data(),
                );
            }}
            SseMessageKind::System => {{
                println!("Heartbeat / system event");
            }}
            SseMessageKind::Retry => {{
                println!("Retry directive: {{:?}}", event.retry());
            }}
            _ => {{}}
        }}
    }}

    // Close when done
    handle.close().await?;
    "#
    );

    println!();
    println!("SseMessageKind variants:");
    println!("  - {}: regular data events", SseMessageKind::Data);
    println!(
        "  - {}: heartbeat / control signals",
        SseMessageKind::System
    );
    println!("  - {}: server retry directives", SseMessageKind::Retry);
    println!("  - {}: unclassified events", SseMessageKind::Unknown);
}
