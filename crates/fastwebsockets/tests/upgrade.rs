// https://github.com/de-vri-es/hyper-tungstenite-rs/tree/main/tests

use http_body_util::Empty;
use hyper::Request;
use hyper::Response;
use hyper::body::Bytes;
use hyper::body::Incoming;
use hyper::header::CONNECTION;
use hyper::header::UPGRADE;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use std::future::Future;
use std::net::Ipv6Addr;
use tokio::net::TcpStream;

use assert2::assert;

struct TestExecutor;

impl<Fut> hyper::rt::Executor<Fut> for TestExecutor
where
  Fut: Future + Send + 'static,
  Fut::Output: Send + 'static,
{
  fn execute(&self, fut: Fut) {
    tokio::spawn(fut);
  }
}

#[tokio::test]
async fn hyper() {
  // Bind a TCP listener to an ephemeral port.
  assert!(
    let Ok(listener) =
      tokio::net::TcpListener::bind((Ipv6Addr::LOCALHOST, 0u16)).await
  );
  assert!(let Ok(bind_addr) = listener.local_addr());

  // Spawn the server in a task.
  tokio::spawn(async move {
    loop {
      let (stream, _) = listener.accept().await.unwrap();
      let io = TokioIo::new(stream);

      tokio::spawn(async move {
        if let Err(err) = http1::Builder::new()
          .serve_connection(io, service_fn(upgrade_websocket))
          .with_upgrades()
          .await
        {
          println!("Error serving connection: {:?}", err);
        }
      });
    }
  });

  // Try to create a websocket connection with the server.
  assert!(let Ok(stream) = TcpStream::connect(bind_addr).await);
  assert!(
    let Ok(req) = Request::builder()
      .method("GET")
      .uri("ws://localhost/foo")
      .header("Host", "localhost")
      .header(UPGRADE, "websocket")
      .header(CONNECTION, "upgrade")
      .header(
        "Sec-WebSocket-Key",
        hpx_fastwebsockets::handshake::generate_key(),
      )
      .header("Sec-WebSocket-Version", "13")
      .body(Empty::<Bytes>::new())
  );
  assert!(let Ok((mut stream, _response)) = hpx_fastwebsockets::handshake::client(&TestExecutor, req, stream).await);

  assert!(let Ok(message) = stream.read_frame().await);
  assert!(message.opcode == hpx_fastwebsockets::OpCode::Text);
  assert!(message.payload == b"Hello!");

  assert!(
    let Ok(()) = stream
      .write_frame(hpx_fastwebsockets::Frame::text(b"Goodbye!".to_vec().into()))
      .await
  );
  assert!(let Ok(close_frame) = stream.read_frame().await);
  assert!(close_frame.opcode == hpx_fastwebsockets::OpCode::Close);
}

async fn upgrade_websocket(
  mut request: Request<Incoming>,
) -> Result<Response<Empty<Bytes>>, hpx_fastwebsockets::WebSocketError> {
  assert!(hpx_fastwebsockets::upgrade::is_upgrade_request(&request) == true);

  let (response, stream) = hpx_fastwebsockets::upgrade::upgrade(&mut request)?;
  tokio::spawn(async move {
    assert!(let Ok(mut stream) = stream.await);
    assert!(let Ok(()) = stream.write_frame(hpx_fastwebsockets::Frame::text(b"Hello!".to_vec().into())).await);
    assert!(let Ok(reply) = stream.read_frame().await);
    assert!(reply.opcode == hpx_fastwebsockets::OpCode::Text);
    assert!(reply.payload == b"Goodbye!");

    assert!(let Ok(()) = stream.write_frame(hpx_fastwebsockets::Frame::close_raw(vec![].into())).await);
  });

  Ok(response)
}
