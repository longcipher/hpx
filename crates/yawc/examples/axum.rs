// Required imports for axum web framework, futures, and websocket operations
use axum::{Router, response::IntoResponse, routing::get};
use futures::SinkExt;
use hpx_yawc::{Frame, OpCode};

// Main entry point using tokio async runtime
#[tokio::main]
async fn main() {
    // Create a new router that handles websocket connections at the root path
    let app = Router::new().route("/", get(ws_handler));

    // Bind TCP listener to all interfaces on port 3000
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

// Handler function for processing individual websocket client connections
async fn handle_client(fut: hpx_yawc::UpgradeFut) -> hpx_yawc::Result<()> {
    // Wait for the websocket connection to be established
    let mut ws = fut.await?;

    println!("Client accepted");

    // Continuously process incoming websocket frames
    loop {
        let frame = ws.next_frame().await?;
        match frame.opcode() {
            // For text or binary frames, echo them back to the client
            OpCode::Text | OpCode::Binary => {
                ws.send(frame).await?;
            }
            // Ignore other types of frames
            _ => {}
        }
    }
}

// Handler for upgrading HTTP connections to websocket connections
async fn ws_handler(
    headers: axum::http::HeaderMap,
    ws: hpx_yawc::IncomingUpgrade,
) -> impl IntoResponse {
    if headers.contains_key("XAUTH") {
        println!("Client is authenticated");
    }

    // Configure websocket options with best compression
    let options =
        hpx_yawc::Options::default().with_compression_level(hpx_yawc::CompressionLevel::best());
    // Upgrade the connection to websocket protocol
    let (response, fut) = ws.upgrade(options).unwrap();
    // Spawn a new task to handle the websocket connection
    tokio::task::spawn(async move {
        if let Err(e) = handle_client(fut).await {
            eprintln!("Error in websocket connection: {}", e);
        }
    });

    response
}
