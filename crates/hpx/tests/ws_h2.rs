#![allow(missing_docs)]
#![cfg(all(feature = "ws-yawc", feature = "http2"))]
#![allow(
    clippy::allow_attributes,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::unwrap_used
)]

//! WebSocket over HTTP/2 (RFC 8441) integration tests.
//!
//! The test server below speaks raw HTTP/2 through the `http2` crate and
//! implements the server half of the RFC 8441 handshake: it advertises
//! `SETTINGS_ENABLE_CONNECT_PROTOCOL`, accepts an extended CONNECT with
//! `:protocol = websocket`, answers `200`, and then echoes WebSocket frames
//! (parsed and re-encoded by a minimal in-test codec, with optional
//! permessage-deflate support).

use std::net::SocketAddr;

use bytes::{Buf as _, Bytes, BytesMut};
use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress};
use http2::{
    ext::Protocol,
    server::{self, SendResponse},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

/// A parsed WebSocket frame (test codec).
struct TestFrame {
    fin: bool,
    opcode: u8,
    rsv1: bool,
    payload: Vec<u8>,
}

/// Parses one frame from the buffer, if complete.
fn parse_frame(buf: &mut BytesMut) -> Option<TestFrame> {
    if buf.len() < 2 {
        return None;
    }
    let fin = buf[0] & 0x80 != 0;
    let rsv1 = buf[0] & 0x40 != 0;
    let opcode = buf[0] & 0x0f;
    let masked = buf[1] & 0x80 != 0;
    let mut len = (buf[1] & 0x7f) as usize;
    let mut offset = 2;
    if len == 126 {
        if buf.len() < 4 {
            return None;
        }
        len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
        offset = 4;
    } else if len == 127 {
        if buf.len() < 10 {
            return None;
        }
        len = u64::from_be_bytes(buf[2..10].try_into().unwrap()) as usize;
        offset = 10;
    }
    let mask_len = if masked { 4 } else { 0 };
    if buf.len() < offset + mask_len + len {
        return None;
    }
    let mut payload = buf[offset + mask_len..offset + mask_len + len].to_vec();
    if masked {
        let mask = &buf[offset..offset + 4];
        for (i, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[i % 4];
        }
    }
    buf.advance(offset + mask_len + len);
    Some(TestFrame {
        fin,
        opcode,
        rsv1,
        payload,
    })
}

/// Encodes an unmasked server-to-client frame.
fn encode_frame(fin: bool, opcode: u8, rsv1: bool, payload: &[u8]) -> Bytes {
    let mut out = Vec::with_capacity(2 + payload.len());
    out.push((u8::from(fin) << 7) | (u8::from(rsv1) << 6) | (opcode & 0x0f));
    if payload.len() < 126 {
        out.push(payload.len() as u8);
    } else if payload.len() < 65536 {
        out.push(126);
        out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        out.push(127);
        out.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    out.extend_from_slice(payload);
    Bytes::from(out)
}

/// Raw-deflates a message payload (server side, no context takeover).
fn deflate_message(payload: &[u8]) -> Vec<u8> {
    let mut compressor = Compress::new(Compression::default(), false);
    let mut out = Vec::with_capacity(payload.len() + 16);
    compressor
        .compress_vec(payload, &mut out, FlushCompress::Sync)
        .unwrap();
    // Strip the trailing empty block the synchronisation flush appends.
    if out.ends_with(&[0x00, 0x00, 0xff, 0xff]) {
        out.truncate(out.len() - 4);
    }
    out
}

/// Inflates a message payload (server side, no context takeover).
fn inflate_message(payload: &[u8]) -> Vec<u8> {
    use flate2::Status;

    let mut framed = payload.to_vec();
    framed.extend_from_slice(&[0x00, 0x00, 0xff, 0xff]);
    let mut decompressor = Decompress::new(false);
    let mut out = Vec::with_capacity(framed.len() * 4);
    let mut input = framed.as_slice();
    loop {
        out.reserve(1024);
        match decompressor
            .decompress_vec(input, &mut out, FlushDecompress::Sync)
            .unwrap()
        {
            Status::StreamEnd => break,
            Status::Ok => {
                let consumed = decompressor.total_in() as usize;
                if consumed >= framed.len() || out.len() > 16 * 1024 * 1024 {
                    break;
                }
                input = &framed[consumed..];
            }
            Status::BufError => {}
        }
    }
    out
}

/// Configuration for the test server.
#[derive(Clone)]
struct ServerConfig {
    /// Whether to advertise `SETTINGS_ENABLE_CONNECT_PROTOCOL`.
    enable_connect: bool,
    /// Subprotocols the server supports, in preference order.
    protocols: Vec<String>,
    /// Whether to accept permessage-deflate (no context takeover).
    deflate: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            enable_connect: true,
            protocols: Vec::new(),
            deflate: false,
        }
    }
}

/// A running RFC 8441 test server.
struct H2WsServer {
    addr: SocketAddr,
}

impl H2WsServer {
    async fn start(config: ServerConfig) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let config = config.clone();
                tokio::spawn(async move {
                    serve_connection(stream, config).await;
                });
            }
        });

        Self { addr }
    }

    fn ws_url(&self, path: &str) -> String {
        format!("ws://{}{path}", self.addr)
    }
}

async fn serve_connection(stream: TcpStream, config: ServerConfig) {
    let mut builder = server::Builder::new();
    if config.enable_connect {
        builder.enable_connect_protocol();
    }
    let mut conn = match builder.handshake::<_, Bytes>(stream).await {
        Ok(conn) => conn,
        Err(_) => return,
    };

    while let Some(accepted) = conn.accept().await {
        let Ok((request, respond)) = accepted else {
            break;
        };
        let config = config.clone();
        tokio::spawn(async move {
            serve_stream(request, respond, config).await;
        });
    }
}

async fn serve_stream(
    request: http::Request<http2::RecvStream>,
    mut respond: SendResponse<Bytes>,
    config: ServerConfig,
) {
    // The extended CONNECT handshake (RFC 8441 §4-5).
    if request.method() != http::Method::CONNECT {
        let _ = respond.send_response(
            http::Response::builder().status(405).body(()).unwrap(),
            true,
        );
        return;
    }
    let is_websocket = request
        .extensions()
        .get::<Protocol>()
        .is_some_and(|protocol| protocol.as_str() == "websocket");
    if !is_websocket {
        let _ = respond.send_response(
            http::Response::builder().status(400).body(()).unwrap(),
            true,
        );
        return;
    }

    // Subprotocol selection: the first server-supported protocol the client
    // also offered wins.
    let offered: Vec<String> = request
        .headers()
        .get("sec-websocket-protocol")
        .and_then(|value| value.to_str().ok())
        .map_or_else(Vec::new, |value| {
            value
                .split(',')
                .map(|part| part.trim().to_owned())
                .collect()
        });
    let selected = config
        .protocols
        .iter()
        .find(|supported| {
            offered
                .iter()
                .any(|offer| offer.eq_ignore_ascii_case(supported))
        })
        .cloned();

    let client_deflate = request.headers().contains_key("sec-websocket-extensions");
    let mut builder = http::Response::builder().status(200);
    if let Some(selected) = &selected {
        builder = builder.header("sec-websocket-protocol", selected);
    }
    if config.deflate && client_deflate {
        builder = builder.header(
            "sec-websocket-extensions",
            "permessage-deflate; server_no_context_takeover; client_no_context_takeover",
        );
    }
    let response = builder.body(()).unwrap();

    let mut send = match respond.send_response(response, false) {
        Ok(send) => send,
        Err(_) => return,
    };
    let mut recv = request.into_body();

    // Echo loop over the WebSocket frames carried by the stream.
    let mut buf = BytesMut::new();
    loop {
        match recv.data().await {
            Some(Ok(data)) => {
                buf.extend_from_slice(&data);
                while let Some(frame) = parse_frame(&mut buf) {
                    // Control frames are echoed uncompressed.
                    let is_control = frame.opcode >= 0x8;
                    let mut payload = frame.payload;
                    let rsv1 = if config.deflate && client_deflate && !is_control {
                        if frame.rsv1 {
                            payload = inflate_message(&payload);
                        }
                        payload = deflate_message(&payload);
                        true
                    } else {
                        false
                    };
                    let encoded = encode_frame(frame.fin, frame.opcode, rsv1, &payload);
                    if send.send_data(encoded, false).is_err() {
                        return;
                    }
                    // A close frame ends the stream after the echo.
                    if frame.opcode == 0x8 {
                        let _ = send.send_data(Bytes::new(), true);
                        return;
                    }
                }
            }
            _ => return,
        }
    }
}

#[tokio::test]
async fn h2_text_echo() {
    let server = H2WsServer::start(ServerConfig::default()).await;

    let response = hpx::Client::new()
        .websocket(server.ws_url("/chat"))
        .force_http2()
        .send()
        .await
        .expect("RFC 8441 handshake should succeed");

    assert_eq!(response.version(), http::Version::HTTP_2);
    assert_eq!(response.status(), http::StatusCode::OK);
    assert!(!response.compression_accepted());
    assert_eq!(response.protocol(), None);

    let mut ws = response
        .into_websocket()
        .await
        .expect("websocket should upgrade cleanly");
    assert_eq!(ws.protocol(), None);

    ws.send(hpx::ws::Message::text("hello over h2"))
        .await
        .expect("send should succeed");
    let message = ws
        .recv()
        .await
        .expect("message expected")
        .expect("no error");
    assert_eq!(message.into_text().expect("text"), "hello over h2");

    ws.send(hpx::ws::Message::binary(vec![1u8, 2, 3]))
        .await
        .expect("send should succeed");
    let message = ws
        .recv()
        .await
        .expect("message expected")
        .expect("no error");
    assert_eq!(message.into_data(), &[1u8, 2, 3][..]);
}

#[tokio::test]
async fn h2_split_echo() {
    let server = H2WsServer::start(ServerConfig::default()).await;

    let response = hpx::Client::new()
        .websocket(server.ws_url("/chat"))
        .force_http2()
        .send()
        .await
        .expect("RFC 8441 handshake should succeed");

    let (mut write, mut read) = response
        .into_websocket()
        .await
        .expect("websocket should upgrade cleanly")
        .split();

    write
        .send(hpx::ws::Message::text("split over h2"))
        .await
        .expect("send should succeed");
    let message = read
        .recv()
        .await
        .expect("message expected")
        .expect("no error");
    assert_eq!(message.into_text().expect("text"), "split over h2");
}

#[tokio::test]
async fn h2_subprotocol_negotiation() {
    let server = H2WsServer::start(ServerConfig {
        protocols: vec!["superchat".to_owned()],
        ..ServerConfig::default()
    })
    .await;

    let response = hpx::Client::new()
        .websocket(server.ws_url("/chat"))
        .force_http2()
        .protocols(["chat", "superchat"])
        .send()
        .await
        .expect("RFC 8441 handshake should succeed");

    assert_eq!(response.protocol(), Some("superchat"));

    let ws = response
        .into_websocket()
        .await
        .expect("websocket should upgrade cleanly");
    assert_eq!(ws.protocol(), Some("superchat"));
}

#[tokio::test]
async fn h2_deflate_echo() {
    let server = H2WsServer::start(ServerConfig {
        deflate: true,
        ..ServerConfig::default()
    })
    .await;

    let response = hpx::Client::new()
        .websocket(server.ws_url("/chat"))
        .force_http2()
        .deflate_request(true)
        .send()
        .await
        .expect("RFC 8441 handshake should succeed");

    assert!(
        response.compression_accepted(),
        "the server accepted permessage-deflate"
    );

    let mut ws = response
        .into_websocket()
        .await
        .expect("websocket should upgrade cleanly");

    let payload = "compress me over http/2, please. ".repeat(8);
    ws.send(hpx::ws::Message::text(payload.clone()))
        .await
        .expect("send should succeed");
    let message = ws
        .recv()
        .await
        .expect("message expected")
        .expect("no error");
    assert_eq!(message.into_text().expect("text"), payload);
}

#[tokio::test]
async fn h2_close_echo() {
    let server = H2WsServer::start(ServerConfig::default()).await;

    let mut ws = hpx::Client::new()
        .websocket(server.ws_url("/chat"))
        .force_http2()
        .send()
        .await
        .expect("RFC 8441 handshake should succeed")
        .into_websocket()
        .await
        .expect("websocket should upgrade cleanly");

    ws.send(hpx::ws::Message::close(Some(
        hpx::ws::message::CloseFrame {
            code: 1000u16.into(),
            reason: "done".into(),
        },
    )))
    .await
    .expect("close should succeed");
    let message = ws
        .recv()
        .await
        .expect("message expected")
        .expect("no error");
    assert!(
        matches!(message, hpx::ws::Message::Close(_)),
        "expected a close echo, got {message:?}"
    );
}

#[tokio::test]
async fn h2_requires_extended_connect_support() {
    let server = H2WsServer::start(ServerConfig {
        enable_connect: false,
        ..ServerConfig::default()
    })
    .await;

    let err = hpx::Client::new()
        .websocket(server.ws_url("/chat"))
        .force_http2()
        .send()
        .await
        .expect_err("the handshake must fail without the server setting");

    let message = err.to_string();
    assert!(
        message.contains("Extended CONNECT"),
        "unexpected error: {message}"
    );
}

/// A minimal HTTP CONNECT proxy: it answers `200` and pipes bytes to the
/// target H2 server regardless of the requested authority (test only).
struct ConnectProxy {
    addr: SocketAddr,
}

impl ConnectProxy {
    async fn start(target: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            loop {
                let Ok((mut client, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    // Read the CONNECT head.
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        if client.read_exact(&mut byte).await.is_err() {
                            return;
                        }
                        head.extend_from_slice(&byte);
                        if head.len() > 8192 {
                            return;
                        }
                    }
                    let head = String::from_utf8_lossy(&head);
                    if !head.starts_with("CONNECT ") {
                        return;
                    }
                    if client
                        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let Ok(mut target) = TcpStream::connect(target).await else {
                        return;
                    };
                    let _ = tokio::io::copy_bidirectional(&mut client, &mut target).await;
                });
            }
        });

        Self { addr }
    }
}

#[tokio::test]
async fn h2_through_http_proxy() {
    let server = H2WsServer::start(ServerConfig::default()).await;
    let proxy = ConnectProxy::start(server.addr).await;

    let response = hpx::Client::new()
        .websocket("ws://hyper.rs.local/chat")
        .force_http2()
        .proxy(hpx::Proxy::http(format!("http://{}", proxy.addr)).unwrap())
        .send()
        .await
        .expect("RFC 8441 handshake through the proxy should succeed");

    assert_eq!(response.version(), http::Version::HTTP_2);

    let mut ws = response
        .into_websocket()
        .await
        .expect("websocket should upgrade cleanly");
    ws.send(hpx::ws::Message::text("proxied h2"))
        .await
        .expect("send should succeed");
    let message = ws
        .recv()
        .await
        .expect("message expected")
        .expect("no error");
    assert_eq!(message.into_text().expect("text"), "proxied h2");
}

/// WebSocket over TLS + HTTP/2, with a client trust store for the
/// self-signed test certificate.
#[tokio::test]
async fn h2_wss_echo() {
    use std::sync::Arc;

    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

    // 1. A self-signed certificate for the test server.
    let certified_key = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
    let cert_der: CertificateDer<'static> = certified_key.cert.der().clone();
    let key_der: PrivateKeyDer<'static> =
        PrivatePkcs8KeyDer::from(certified_key.signing_key.serialize_der()).into();

    // 2. The TLS server half: accept TCP, run TLS with ALPN `h2`, then serve
    //    the RFC 8441 handshake on the decrypted stream.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut server_tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .unwrap();
    server_tls.alpn_protocols = vec![b"h2".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_tls));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(stream).await else {
                    return;
                };
                serve_connection_tls(tls, ServerConfig::default()).await;
            });
        }
    });

    // 3. The client trusts the self-signed certificate.
    let client_provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut root_store = rustls::RootCertStore::empty();
    root_store.add(cert_der).unwrap();
    let mut client_tls = rustls::ClientConfig::builder_with_provider(client_provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    client_tls.alpn_protocols = vec![b"h2".to_vec()];
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client_tls));

    // 4. Connect through the yawc H2 builder with the custom connector.
    let mut ws = hpx_yawc::WebSocket::connect_h2(format!("wss://{addr}/chat").parse().unwrap())
        .with_options(hpx_yawc::Options::default())
        .with_connector(connector)
        .await
        .expect("RFC 8441 handshake over TLS should succeed");

    use futures_util::{SinkExt, StreamExt};
    ws.send(hpx_yawc::Frame::text("secure h2"))
        .await
        .expect("send should succeed");
    let frame = ws.next().await.expect("frame expected");
    let (opcode, _fin, payload) = frame.into_parts();
    assert_eq!(opcode, hpx_yawc::OpCode::Text);
    assert_eq!(payload.as_ref(), b"secure h2");
}

async fn serve_connection_tls<S>(stream: S, config: ServerConfig)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let mut builder = server::Builder::new();
    if config.enable_connect {
        builder.enable_connect_protocol();
    }
    let mut conn = match builder.handshake::<_, Bytes>(stream).await {
        Ok(conn) => conn,
        Err(_) => return,
    };

    while let Some(accepted) = conn.accept().await {
        let Ok((request, respond)) = accepted else {
            break;
        };
        let config = config.clone();
        tokio::spawn(async move {
            serve_stream(request, respond, config).await;
        });
    }
}
