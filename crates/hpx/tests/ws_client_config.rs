#![allow(missing_docs)]
#![cfg(feature = "ws-yawc")]
#![allow(
    clippy::allow_attributes,
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used
)]

//! The WebSocket handshake runs through the [`Client`] that created the
//! request, so the client's configuration applies to `ws://` and `wss://`:
//! proxies, the TLS trust store, DNS overrides, timeouts and default headers.
//! The upgraded connection then carries the client's frames to the server.

mod support;

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use hpx::{Client, Proxy, tls::CertStore};
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, IsCa, KeyPair, KeyUsagePurpose,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha1::Digest as _;
use support::{server, tunnel_proxy::TunnelProxy};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpListener,
};

/// A host name that does not resolve. Reaching a server under this name
/// proves that the proxy or the DNS override carried the connection.
const HOST: &str = "ws.test";

/// The text frame every test server sends after the handshake.
const GREETING: &str = "hello";

/// Bounds every test, so a handshake that hangs fails instead of stalling.
const TEST_DEADLINE: Duration = Duration::from_secs(10);

/// Reads one HTTP request head (through the blank line).
async fn read_head<S: AsyncRead + Unpin>(stream: &mut S) -> String {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read_exact(&mut byte).await.is_err() {
            break;
        }
        head.push(byte[0]);
    }
    String::from_utf8(head).expect("request head should be valid utf-8")
}

/// The `Sec-WebSocket-Accept` value for a request head (RFC 6455 §4.2.2).
fn websocket_accept_for(head: &str) -> String {
    let key = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("sec-websocket-key")
                .then(|| value.trim().to_owned())
        })
        .expect("upgrade request should carry sec-websocket-key");

    let mut sha1 = sha1::Sha1::new();
    sha1.update(key.as_bytes());
    sha1.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    base64_simd::STANDARD.encode_to_string(&sha1.finalize()[..])
}

/// The `101` response that accepts the upgrade request `head`.
fn upgrade_response(head: &str) -> String {
    format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Connection: upgrade\r\n\
         Upgrade: websocket\r\n\
         Sec-WebSocket-Accept: {}\r\n\
         \r\n",
        websocket_accept_for(head)
    )
}

/// A client frame's opcode and unmasked payload; `None` when the client
/// went away instead.
type ClientFrame = Option<(u8, Vec<u8>)>;

/// Reads one masked client frame with a short payload (RFC 6455 §5.2), and
/// returns its opcode and unmasked payload. `None` means the client went away.
async fn read_client_frame<S: AsyncRead + Unpin>(stream: &mut S) -> ClientFrame {
    let mut head = [0u8; 2];
    stream.read_exact(&mut head).await.ok()?;
    assert_ne!(head[1] & 0x80, 0, "client frames must be masked");
    let len = usize::from(head[1] & 0x7F);
    assert!(len < 126, "test frames carry short payloads");
    let mut mask = [0u8; 4];
    stream.read_exact(&mut mask).await.ok()?;
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await.ok()?;
    for (i, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[i % 4];
    }
    Some((head[0] & 0x0F, payload))
}

/// Completes the upgrade, records the request head, sends [`GREETING`] as a
/// text frame, then holds the connection open until the client goes away.
async fn serve_upgrade<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    recorded: Arc<Mutex<Vec<String>>>,
) {
    let head = read_head(&mut stream).await;
    let response = upgrade_response(&head);
    recorded.lock().unwrap().push(head);
    stream.write_all(response.as_bytes()).await.unwrap();
    let mut frame = vec![0x81, u8::try_from(GREETING.len()).unwrap()];
    frame.extend_from_slice(GREETING.as_bytes());
    stream.write_all(&frame).await.unwrap();
    stream.flush().await.unwrap();

    let mut drained = Vec::new();
    let _ = stream.read_to_end(&mut drained).await;
}

/// A private certificate authority and a server certificate it issued for
/// [`HOST`].
struct PrivatePki {
    ca: CertificateDer<'static>,
    server_cert: CertificateDer<'static>,
    server_key: PrivateKeyDer<'static>,
}

impl PrivatePki {
    fn new() -> Self {
        // Distinct subject names: a leaf whose subject equals its issuer's
        // reads as self-signed.
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "hpx test CA");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let ca = CertifiedIssuer::self_signed(ca_params, KeyPair::generate().unwrap()).unwrap();

        let server_key = KeyPair::generate().unwrap();
        let mut server_params = CertificateParams::new(vec![HOST.to_owned()]).unwrap();
        server_params
            .distinguished_name
            .push(DnType::CommonName, HOST);
        let server_cert = server_params.signed_by(&server_key, &ca).unwrap();

        Self {
            ca: ca.der().clone(),
            server_cert: server_cert.der().clone(),
            server_key: PrivatePkcs8KeyDer::from(server_key.serialize_der()).into(),
        }
    }

    /// A client trust store that holds only the private CA.
    fn cert_store(&self) -> CertStore {
        CertStore::from_der_certs([self.ca.as_ref()]).unwrap()
    }
}

/// A WebSocket server on an ephemeral loopback port.
struct WsServer {
    addr: SocketAddr,
    heads: Arc<Mutex<Vec<String>>>,
}

impl WsServer {
    /// Serves plain `ws://`.
    async fn plain() -> Self {
        Self::start(None).await
    }

    /// Serves `wss://` with the server certificate of `pki`.
    async fn tls(pki: &PrivatePki) -> Self {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![pki.server_cert.clone()], pki.server_key.clone_key())
            .unwrap();
        Self::start(Some(tokio_rustls::TlsAcceptor::from(Arc::new(config)))).await
    }

    async fn start(tls: Option<tokio_rustls::TlsAcceptor>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let heads = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&heads);

        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let tls = tls.clone();
                let recorded = Arc::clone(&recorded);
                tokio::spawn(async move {
                    match tls {
                        Some(acceptor) => {
                            if let Ok(stream) = acceptor.accept(stream).await {
                                serve_upgrade(stream, recorded).await;
                            }
                        }
                        None => serve_upgrade(stream, recorded).await,
                    }
                });
            }
        });

        Self { addr, heads }
    }

    /// The request heads received so far.
    fn heads(&self) -> Vec<String> {
        self.heads.lock().unwrap().clone()
    }
}

/// Opens a WebSocket to `url` and returns the first text message.
async fn first_text(client: &Client, url: &str) -> String {
    let response = tokio::time::timeout(TEST_DEADLINE, client.websocket(url).send())
        .await
        .expect("the handshake must not hang")
        .expect("websocket handshake should succeed");
    let mut ws = response
        .into_websocket()
        .await
        .expect("websocket should upgrade cleanly");
    let message = tokio::time::timeout(TEST_DEADLINE, ws.recv())
        .await
        .expect("the greeting must arrive")
        .expect("message expected")
        .expect("no error");
    message.into_text().expect("text message").to_string()
}

/// A client-level HTTP proxy tunnels `ws://` with `CONNECT` (RFC 6455 §4.1).
#[tokio::test]
async fn client_http_proxy_tunnels_ws() {
    let server = WsServer::plain().await;
    let proxy = TunnelProxy::http(server.addr).await;
    let client = Client::builder()
        .proxy(Proxy::all(format!("http://{}", proxy.addr())).unwrap())
        .build()
        .unwrap();

    let text = first_text(&client, &format!("ws://{HOST}/")).await;

    assert_eq!(text, GREETING);
    assert_eq!(proxy.destinations(), [format!("{HOST}:80")]);
}

/// A client-level HTTP proxy tunnels `wss://`; TLS to the origin runs inside
/// the tunnel and is verified against the client's trust store.
#[tokio::test]
async fn client_http_proxy_tunnels_wss() {
    let pki = PrivatePki::new();
    let server = WsServer::tls(&pki).await;
    let proxy = TunnelProxy::http(server.addr).await;
    let client = Client::builder()
        .proxy(Proxy::all(format!("http://{}", proxy.addr())).unwrap())
        .cert_store(pki.cert_store())
        .build()
        .unwrap();

    let text = first_text(&client, &format!("wss://{HOST}/")).await;

    assert_eq!(text, GREETING);
    assert_eq!(proxy.destinations(), [format!("{HOST}:443")]);
}

/// A client-level `socks5h://` proxy carries `ws://`, and resolves the host.
#[cfg(feature = "socks")]
#[tokio::test]
async fn client_socks5h_proxy_carries_ws() {
    let server = WsServer::plain().await;
    let proxy = TunnelProxy::socks5(server.addr).await;
    let client = Client::builder()
        .proxy(Proxy::all(format!("socks5h://{}", proxy.addr())).unwrap())
        .build()
        .unwrap();

    let text = first_text(&client, &format!("ws://{HOST}/")).await;

    assert_eq!(text, GREETING);
    assert_eq!(proxy.destinations(), [format!("{HOST}:80")]);
}

/// A client-level `socks5h://` proxy carries `wss://`.
#[cfg(feature = "socks")]
#[tokio::test]
async fn client_socks5h_proxy_carries_wss() {
    let pki = PrivatePki::new();
    let server = WsServer::tls(&pki).await;
    let proxy = TunnelProxy::socks5(server.addr).await;
    let client = Client::builder()
        .proxy(Proxy::all(format!("socks5h://{}", proxy.addr())).unwrap())
        .cert_store(pki.cert_store())
        .build()
        .unwrap();

    let text = first_text(&client, &format!("wss://{HOST}/")).await;

    assert_eq!(text, GREETING);
    assert_eq!(proxy.destinations(), [format!("{HOST}:443")]);
}

/// A private CA in the client's `cert_store` validates a `wss://` server,
/// reached through the client's DNS override; without that CA the same
/// handshake fails certificate verification.
#[tokio::test]
async fn cert_store_private_ca_validates_wss() {
    let pki = PrivatePki::new();
    let server = WsServer::tls(&pki).await;
    let url = format!("wss://{HOST}:{}/", server.addr.port());

    let trusting = Client::builder()
        .no_proxy()
        .resolve(HOST, server.addr)
        .cert_store(pki.cert_store())
        .build()
        .unwrap();
    assert_eq!(first_text(&trusting, &url).await, GREETING);

    let untrusting = Client::builder()
        .no_proxy()
        .resolve(HOST, server.addr)
        .build()
        .unwrap();
    let err = tokio::time::timeout(TEST_DEADLINE, untrusting.websocket(&url).send())
        .await
        .expect("the handshake must not hang")
        .expect_err("a certificate from an unknown CA must be rejected");
    assert!(err.is_connect(), "unexpected error: {err:?}");
}

/// Starts a server that accepts TCP and reads the upgrade request but never
/// answers it.
fn silent_server() -> server::Server {
    server::low_level_with_response(|_request, _stream| Box::new(std::future::pending()))
}

/// The client's connect timeout bounds the whole opening handshake: a server
/// that accepts the connection but never answers the upgrade fails it, even
/// when the request timeout is far longer.
#[tokio::test]
async fn connect_timeout_bounds_an_unanswered_upgrade() {
    let server = silent_server();
    let client = Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_millis(200))
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap();

    let err = tokio::time::timeout(
        TEST_DEADLINE,
        client.websocket(format!("ws://{}/", server.addr())).send(),
    )
    .await
    .expect("the connect timeout must end the handshake")
    .expect_err("an unanswered upgrade must fail");
    assert!(err.is_timeout(), "unexpected error: {err:?}");
}

/// The client's request timeout bounds the opening handshake too.
#[tokio::test]
async fn request_timeout_bounds_an_unanswered_upgrade() {
    let server = silent_server();
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(200))
        .build()
        .unwrap();

    let err = tokio::time::timeout(
        TEST_DEADLINE,
        client.websocket(format!("ws://{}/", server.addr())).send(),
    )
    .await
    .expect("the request timeout must end the handshake")
    .expect_err("an unanswered upgrade must fail");
    assert!(err.is_timeout(), "unexpected error: {err:?}");
}

/// The client's default headers are sent with the upgrade request.
#[tokio::test]
async fn default_headers_reach_the_upgrade_request() {
    let server = WsServer::plain().await;
    let mut headers = http::HeaderMap::new();
    headers.insert(
        "x-client-default",
        http::HeaderValue::from_static("present"),
    );
    let client = Client::builder()
        .no_proxy()
        .default_headers(headers)
        .build()
        .unwrap();

    let text = first_text(&client, &format!("ws://{}/", server.addr)).await;
    assert_eq!(text, GREETING);

    // The server records the head before it answers the upgrade.
    let heads = server.heads();
    assert_eq!(heads.len(), 1);
    assert!(
        heads[0].contains("x-client-default: present"),
        "the default header must be sent: {}",
        heads[0]
    );
}

/// Starts a server that accepts the upgrade and reports the first two frames
/// the client sends; `None` reports that the client went away instead.
fn frame_recording_server() -> (server::Server, std::sync::mpsc::Receiver<ClientFrame>) {
    let (frames_tx, frames_rx) = std::sync::mpsc::channel();
    let server = server::low_level_with_response(move |request, stream| {
        let head = std::str::from_utf8(request)
            .expect("request head should be valid utf-8")
            .to_owned();
        let frames_tx = frames_tx.clone();
        Box::new(async move {
            stream
                .write_all(upgrade_response(&head).as_bytes())
                .await
                .unwrap();
            for _ in 0..2 {
                let _ = frames_tx.send(read_client_frame(stream).await);
            }
        })
    });
    (server, frames_rx)
}

/// The frames a client sends for `send(text("ping"))` then `close(1000, "bye")`.
fn ping_then_close() -> [ClientFrame; 2] {
    let mut close = 1000u16.to_be_bytes().to_vec();
    close.extend_from_slice(b"bye");
    [Some((0x1, b"ping".to_vec())), Some((0x8, close))]
}

/// Opens a WebSocket to the frame-recording `server`.
async fn connect(server: &server::Server) -> hpx::ws::WebSocket {
    let client = Client::builder().no_proxy().build().unwrap();
    tokio::time::timeout(
        TEST_DEADLINE,
        client.websocket(format!("ws://{}/", server.addr())).send(),
    )
    .await
    .expect("the handshake must not hang")
    .expect("websocket handshake should succeed")
    .into_websocket()
    .await
    .expect("websocket should upgrade cleanly")
}

/// Receives the two frames the server reports.
fn received(frames: &std::sync::mpsc::Receiver<ClientFrame>) -> [ClientFrame; 2] {
    std::array::from_fn(|_| {
        frames
            .recv_timeout(TEST_DEADLINE)
            .expect("the server must report each frame")
    })
}

/// `send` and `close` on an unsplit WebSocket put their frames on the wire.
#[tokio::test]
async fn unsplit_send_and_close_reach_the_server() {
    let (server, frames) = frame_recording_server();
    let mut ws = connect(&server).await;

    ws.send(hpx::ws::Message::text("ping")).await.unwrap();
    ws.close(hpx::ws::message::CloseCode::NORMAL, "bye")
        .await
        .unwrap();

    assert_eq!(received(&frames), ping_then_close());
}

/// `send` and `close` on the write half of a split WebSocket put their frames
/// on the wire.
#[tokio::test]
async fn split_send_and_close_reach_the_server() {
    let (server, frames) = frame_recording_server();
    let (mut write, _read) = connect(&server).await.split();

    write.send(hpx::ws::Message::text("ping")).await.unwrap();
    write
        .close(hpx::ws::message::CloseCode::NORMAL, "bye")
        .await
        .unwrap();

    assert_eq!(received(&frames), ping_then_close());
}
