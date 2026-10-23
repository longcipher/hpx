//! WebSocket over HTTP/2 (RFC 8441) integration tests.
//!
//! These tests exercise the public [`hpx_yawc`](crate) H2 API against a
//! minimal raw-`http2` test server: the extended-CONNECT handshake, frame
//! echo, subprotocol negotiation, and the missing-server-setting error.

#![cfg(feature = "h2")]
#![allow(
    clippy::allow_attributes,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::unwrap_used,
    missing_docs
)]

use std::net::SocketAddr;

use bytes::{Buf as _, Bytes, BytesMut};
use futures::{SinkExt, StreamExt};
use http2::{
    ext::Protocol,
    server::{self, SendResponse},
};
use tokio::net::{TcpListener, TcpStream};

/// Parses one client-to-server frame from the buffer, if complete.
fn parse_frame(buf: &mut BytesMut) -> Option<(u8, Vec<u8>)> {
    if buf.len() < 2 {
        return None;
    }
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
    Some((opcode, payload))
}

/// Encodes an unmasked server-to-client text frame.
fn encode_text(payload: &[u8]) -> Bytes {
    encode_frame(0x1, payload)
}

/// Encodes an unmasked server-to-client frame.
fn encode_frame(opcode: u8, payload: &[u8]) -> Bytes {
    let mut out = Vec::with_capacity(2 + payload.len());
    out.push(0x80 | (opcode & 0x0f));
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

struct TestServer {
    addr: SocketAddr,
}

impl TestServer {
    /// Starts the server.
    ///
    /// When `enable_connect` is false, the server does not advertise
    /// `SETTINGS_ENABLE_CONNECT_PROTOCOL`.
    async fn start(enable_connect: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    serve(stream, enable_connect).await;
                });
            }
        });

        Self { addr }
    }
}

async fn serve(stream: TcpStream, enable_connect: bool) {
    let mut builder = server::Builder::new();
    if enable_connect {
        builder.enable_connect_protocol();
    }
    let mut conn = match builder.handshake::<_, Bytes>(stream).await {
        Ok(conn) => conn,
        Err(_) => return,
    };

    while let Some(accepted) = conn.accept().await {
        let Ok((request, mut respond)) = accepted else {
            break;
        };
        tokio::spawn(async move {
            serve_stream(request, &mut respond).await;
        });
    }
}

async fn serve_stream(
    request: http::Request<http2::RecvStream>,
    respond: &mut SendResponse<Bytes>,
) {
    if request.method() != http::Method::CONNECT
        || request
            .extensions()
            .get::<Protocol>()
            .is_none_or(|protocol| protocol.as_str() != "websocket")
    {
        let _ = respond.send_response(
            http::Response::builder().status(400).body(()).unwrap(),
            true,
        );
        return;
    }

    // Select the `chat` subprotocol when the client offers it.
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
    let mut builder = http::Response::builder().status(200);
    if offered.iter().any(|offer| offer == "chat") {
        builder = builder.header("sec-websocket-protocol", "chat");
    }
    let response = builder.body(()).unwrap();

    let mut send = match respond.send_response(response, false) {
        Ok(send) => send,
        Err(_) => return,
    };
    let mut recv = request.into_body();

    let mut buf = BytesMut::new();
    loop {
        match recv.data().await {
            Some(Ok(data)) => {
                // The chunk is copied into the server buffer, freeing the
                // HTTP/2 receive window.
                let _ = recv.flow_control().release_capacity(data.len());
                buf.extend_from_slice(&data);
                while let Some((opcode, payload)) = parse_frame(&mut buf) {
                    // Echo text and binary frames in frame-sized pieces,
                    // preserving the opcode; answer close frames and end
                    // the stream.
                    if opcode == 0x1 || opcode == 0x2 {
                        for chunk in encode_frame(opcode, &payload).chunks(8192) {
                            if send
                                .send_data(Bytes::copy_from_slice(chunk), false)
                                .is_err()
                            {
                                return;
                            }
                        }
                    } else if opcode == 0x8 {
                        let _ = send.send_data(encode_text(&payload), false);
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
async fn h2_echo_through_the_public_api() {
    let server = TestServer::start(true).await;

    let mut ws =
        hpx_yawc::WebSocket::connect_h2(format!("ws://{}/chat", server.addr).parse().unwrap())
            .with_options(hpx_yawc::Options::default())
            .await
            .expect("RFC 8441 handshake should succeed");

    ws.send(hpx_yawc::Frame::text("hello"))
        .await
        .expect("send should succeed");
    let frame = ws.next().await.expect("frame expected");
    let (opcode, _fin, payload) = frame.into_parts();
    assert_eq!(opcode, hpx_yawc::OpCode::Text);
    assert_eq!(payload.as_ref(), b"hello");
}

#[tokio::test]
async fn h2_negotiates_a_subprotocol() {
    let server = TestServer::start(true).await;

    let ws = hpx_yawc::WebSocket::connect_h2(format!("ws://{}/chat", server.addr).parse().unwrap())
        .with_options(hpx_yawc::Options::default().with_protocols(["meta", "chat"]))
        .await
        .expect("RFC 8441 handshake should succeed");

    assert_eq!(ws.protocol(), Some("chat"));
}

#[tokio::test]
async fn h2_fails_without_the_server_setting() {
    let server = TestServer::start(false).await;

    let result =
        hpx_yawc::WebSocket::connect_h2(format!("ws://{}/chat", server.addr).parse().unwrap())
            .await;
    let err = match result {
        Ok(_) => panic!("the handshake must fail without the server setting"),
        Err(err) => err,
    };

    assert!(err.is_handshake_error(), "unexpected error: {err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h2_echoes_a_large_message() {
    let server = TestServer::start(true).await;

    let mut ws =
        hpx_yawc::WebSocket::connect_h2(format!("ws://{}/chat", server.addr).parse().unwrap())
            .with_options(
                hpx_yawc::Options::default()
                    .with_max_payload_read(1024 * 1024)
                    .with_max_read_buffer(2 * 1024 * 1024),
            )
            .await
            .expect("RFC 8441 handshake should succeed");

    // Larger than any single read buffer, so the transport must handle
    // partial reads and exact write lengths. Bounded so a corrupted
    // stream fails instead of hanging.
    let payload = "0123456789abcdef".repeat(16 * 1024);
    ws.send(hpx_yawc::Frame::binary(payload.clone()))
        .await
        .expect("send should succeed");
    let frame = tokio::time::timeout(std::time::Duration::from_secs(20), ws.next())
        .await
        .expect("the echo arrives")
        .expect("frame expected");
    let (opcode, _fin, received) = frame.into_parts();
    assert_eq!(opcode, hpx_yawc::OpCode::Binary);
    assert_eq!(received.as_ref(), payload.as_bytes());
}

#[tokio::test]
async fn h2_wss_echo_with_custom_trust() {
    use std::sync::Arc;

    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

    let certified_key = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
    let cert_der: CertificateDer<'static> = certified_key.cert.der().clone();
    let key_der: PrivateKeyDer<'static> =
        PrivatePkcs8KeyDer::from(certified_key.signing_key.serialize_der()).into();

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
                serve_tls(tls).await;
            });
        }
    });

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

    let mut ws = hpx_yawc::WebSocket::connect_h2(format!("wss://{addr}/chat").parse().unwrap())
        .with_connector(connector)
        .await
        .expect("RFC 8441 handshake over TLS should succeed");

    ws.send(hpx_yawc::Frame::text("secure"))
        .await
        .expect("send should succeed");
    let frame = ws.next().await.expect("frame expected");
    let (opcode, _fin, payload) = frame.into_parts();
    assert_eq!(opcode, hpx_yawc::OpCode::Text);
    assert_eq!(payload.as_ref(), b"secure");
}

async fn serve_tls<S>(stream: S)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let mut builder = server::Builder::new();
    builder.enable_connect_protocol();
    let mut conn = match builder.handshake::<_, Bytes>(stream).await {
        Ok(conn) => conn,
        Err(_) => return,
    };

    while let Some(accepted) = conn.accept().await {
        let Ok((request, mut respond)) = accepted else {
            break;
        };
        tokio::spawn(async move {
            serve_stream(request, &mut respond).await;
        });
    }
}
