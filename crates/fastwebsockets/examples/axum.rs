use axum::{Router, response::IntoResponse, routing::get};
use hpx_fastwebsockets::OpCode;
use hpx_fastwebsockets::WebSocketError;
use hpx_fastwebsockets::upgrade;

#[tokio::main]
async fn main() {
  let app = Router::new().route("/", get(ws_handler));

  let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
  axum::serve(listener, app).await.unwrap();
}

async fn handle_client(fut: upgrade::UpgradeFut) -> Result<(), WebSocketError> {
  let mut ws = hpx_fastwebsockets::FragmentCollector::new(fut.await?);

  loop {
    let frame = ws.read_frame().await?;
    match frame.opcode {
      OpCode::Close => break,
      OpCode::Text | OpCode::Binary => {
        ws.write_frame(frame).await?;
      }
      _ => {}
    }
  }

  Ok(())
}

async fn ws_handler(ws: upgrade::IncomingUpgrade) -> impl IntoResponse {
  let (response, fut) = ws.upgrade().unwrap();
  tokio::task::spawn(async move {
    if let Err(e) = handle_client(fut).await {
      eprintln!("Error in websocket connection: {}", e);
    }
  });
  response
}
