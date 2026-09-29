//! Connect to an SSE endpoint and print events until interrupted.
//!
//! ```not_rust
//! cargo run --example sse_stream --features sse -- https://example.com/events
//! ```

use futures_util::StreamExt;
use hpx::sse::{RequestBuilderSseExt, SseEvent};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://httpbin.org/events/1".to_string());

    println!("connecting to {url} …");
    let mut source = hpx::Client::builder().build()?.get(url).into_event_source();

    while let Some(event) = source.next().await {
        match event? {
            SseEvent::Open => println!("connected"),
            SseEvent::Message(msg) => {
                if let Some(id) = &msg.last_event_id {
                    println!("[{id}] {}: {}", msg.event, msg.data);
                } else {
                    println!("{}: {}", msg.event, msg.data);
                }
            }
            SseEvent::Error(err) => eprintln!("reconnecting: {err}"),
        }
    }

    Ok(())
}
