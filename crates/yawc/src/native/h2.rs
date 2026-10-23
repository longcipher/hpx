//! WebSocket over HTTP/2 (RFC 8441).
//!
//! This module bootstraps a WebSocket connection from a single stream of an
//! HTTP/2 connection using the Extended CONNECT method. The handshake
//! replaces the HTTP/1 upgrade flow:
//!
//! ```text
//! HEADERS: :method = CONNECT, :protocol = websocket,
//!          :scheme, :path, :authority, sec-websocket-*
//! <-- HEADERS: :status = 200, sec-websocket-protocol?
//! <-- DATA: WebSocket frames -->
//! ```
//!
//! Once the server answers with a `2xx` status, the stream carries raw
//! WebSocket frames in both directions and is treated exactly like the TCP
//! connection of RFC 6455. An `END_STREAM` flag is an orderly closure and a
//! `RST_STREAM` is an abort.
//!
//! Cleartext (`ws://`) connections use HTTP/2 with prior knowledge; the
//! HTTP/1.1 Upgrade dance for discovering h2c is out of scope.

use std::{
    fmt,
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use bytes::{Buf as _, Bytes, BytesMut};
use futures::{FutureExt, future::BoxFuture};
use http::{HeaderMap, Uri, header};
use http2::{
    RecvStream, SendStream,
    client::{ResponseFuture, SendRequest},
    ext::Protocol,
};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use url::Url;

#[cfg(feature = "proxy")]
use super::proxy::ProxyConfig;
use super::{
    HttpRequestBuilder, Negotiation, Options, Role, WebSocket, WebSocketError, builder::HttpRequest,
};
use crate::compression::WebSocketExtensions;

/// How long to wait for the server's `SETTINGS_ENABLE_CONNECT_PROTOCOL`.
///
/// Servers send their SETTINGS frame as part of the connection preface, so
/// this is normally already known when the handshake future completes; the
/// wait only covers a late second SETTINGS frame.
const ENABLE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// A single HTTP/2 stream carrying WebSocket frames (RFC 8441).
///
/// The stream behaves like a byte-oriented connection: DATA frames sent by
/// the peer are exposed for reading, and written bytes are sent as DATA
/// frames. The HTTP/2 connection itself is driven by a background task.
pub struct H2Stream {
    send: SendStream<Bytes>,
    recv: RecvStream,
    read_buf: BytesMut,
    eof: bool,
    shutdown: bool,
}

impl fmt::Debug for H2Stream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("H2Stream").finish_non_exhaustive()
    }
}

/// A WebSocket connection bootstrapped from HTTP/2 (RFC 8441).
pub type H2WebSocket = WebSocket<H2Stream>;

impl tokio::io::AsyncRead for H2Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if !self.read_buf.is_empty() {
            let len = self.read_buf.len().min(buf.remaining());
            buf.put_slice(&self.read_buf[..len]);
            self.read_buf.advance(len);
            // The bytes handed to the reader free up receive capacity.
            let _ = self.recv.flow_control().release_capacity(len);
            return Poll::Ready(Ok(()));
        }

        if self.eof {
            return Poll::Ready(Ok(()));
        }

        match self.recv.poll_data(cx) {
            Poll::Ready(Some(Ok(data))) => {
                let len = data.len().min(buf.remaining());
                buf.put_slice(&data[..len]);
                if len < data.len() {
                    self.read_buf.extend_from_slice(&data[len..]);
                }
                // The bytes handed to the reader free up receive capacity.
                let _ = self.recv.flow_control().release_capacity(len);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Some(Err(err))) => {
                Poll::Ready(Err(io::Error::other(format!("h2 stream error: {err}"))))
            }
            Poll::Ready(None) => {
                self.eof = true;
                Poll::Ready(Ok(()))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl tokio::io::AsyncWrite for H2Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.shutdown {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "h2 stream is closed",
            )));
        }
        // `send_data` buffers when no capacity is available, so the write
        // completes synchronously.
        match self.send.send_data(Bytes::copy_from_slice(buf), false) {
            Ok(()) => Poll::Ready(Ok(buf.len())),
            Err(err) => Poll::Ready(Err(io::Error::other(format!("h2 send error: {err}")))),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if !self.shutdown {
            self.shutdown = true;
            if let Err(err) = self.send.send_data(Bytes::new(), true) {
                return Poll::Ready(Err(io::Error::other(format!("h2 shutdown error: {err}"))));
            }
        }
        Poll::Ready(Ok(()))
    }
}

/// Builder for establishing a WebSocket connection over HTTP/2 (RFC 8441).
pub struct H2WebSocketBuilder {
    opts: Option<H2BuilderOpts>,
    future: Option<BoxFuture<'static, std::result::Result<H2WebSocket, WebSocketError>>>,
}

/// Internal options for [`H2WebSocketBuilder`].
struct H2BuilderOpts {
    url: Url,
    establish_options: Option<Options>,
    http_builder: Option<HttpRequestBuilder>,
    connector: Option<TlsConnector>,
    #[cfg(feature = "proxy")]
    proxy: Option<ProxyConfig>,
}

impl WebSocket<H2Stream> {
    /// Starts an RFC 8441 handshake for the given URL.
    ///
    /// `ws://` URLs use cleartext HTTP/2 with prior knowledge; `wss://`
    /// URLs use TLS with ALPN `h2`.
    pub fn connect_h2(url: Url) -> H2WebSocketBuilder {
        H2WebSocketBuilder {
            opts: Some(H2BuilderOpts {
                url,
                establish_options: None,
                http_builder: None,
                connector: None,
                #[cfg(feature = "proxy")]
                proxy: None,
            }),
            future: None,
        }
    }

    /// Wraps a stream whose RFC 8441 handshake already completed.
    ///
    /// The negotiated subprotocol and extensions come from the CONNECT
    /// response headers.
    pub(crate) fn from_h2_stream(
        io: H2Stream,
        options: Options,
        protocol: Option<String>,
        extensions: Option<WebSocketExtensions>,
    ) -> Self {
        let max_read_buffer = options.max_read_buffer.unwrap_or(
            options
                .max_payload_read
                .map(|payload_read| payload_read * 2)
                .unwrap_or(super::MAX_READ_BUFFER),
        );
        let negotiation = Negotiation {
            extensions,
            compression_level: options.compression.as_ref().map(|c| c.level),
            max_payload_read: options.max_payload_read.unwrap_or(super::MAX_PAYLOAD_READ),
            max_read_buffer,
            utf8: options.check_utf8,
            fragmentation: options.fragmentation.clone(),
            max_backpressure_write_boundary: options.max_backpressure_write_boundary,
            protocol,
        };
        Self::new(Role::Client, io, Bytes::new(), negotiation)
    }
}

impl H2WebSocketBuilder {
    /// Sets the WebSocket configuration options.
    pub fn with_options(mut self, options: Options) -> Self {
        if let Some(opts) = &mut self.opts {
            opts.establish_options = Some(options);
        }
        self
    }

    /// Sets a custom TLS connector for `wss://` connections.
    pub fn with_connector(mut self, connector: TlsConnector) -> Self {
        if let Some(opts) = &mut self.opts {
            opts.connector = Some(connector);
        }
        self
    }

    /// Sets a custom HTTP request builder for the CONNECT request.
    ///
    /// Headers set on the builder are sent with the extended CONNECT
    /// request. The `connection`, `upgrade`, `host`, `sec-websocket-key`
    /// and `sec-websocket-accept` headers are managed by the handshake and
    /// are ignored when set here.
    pub fn with_request(mut self, builder: HttpRequestBuilder) -> Self {
        if let Some(opts) = &mut self.opts {
            opts.http_builder = Some(builder);
        }
        self
    }

    /// Sets a proxy for the connection.
    ///
    /// Following RFC 8441 §7, the proxy is tunnelled with a traditional
    /// CONNECT (or SOCKS handshake) first; HTTP/2 runs inside the tunnel.
    #[cfg(feature = "proxy")]
    pub fn with_proxy(mut self, proxy: ProxyConfig) -> Self {
        if let Some(opts) = &mut self.opts {
            opts.proxy = Some(proxy);
        }
        self
    }
}

impl Future for H2WebSocketBuilder {
    type Output = std::result::Result<H2WebSocket, WebSocketError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if let Some(opts) = this.opts.take() {
            this.future = Some(connect_h2_priv(opts).boxed());
        }

        let Some(pinned) = &mut this.future else {
            unreachable!()
        };
        pinned.poll_unpin(cx)
    }
}

/// Formats a `host[:port]` authority value from a bracket-stripped host.
///
/// [`Url::host_str`] strips the brackets from IPv6 literals, so they are
/// re-added here as RFC 3986 §3.2.2 requires — both for the `:authority`
/// pseudo-header and for dialing.
fn format_authority(host: &str, port: Option<u16>) -> String {
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    match port {
        Some(port) => format!("{host}:{port}"),
        None => host,
    }
}

/// Builds the extended CONNECT request (RFC 8441 §5).
///
/// The request carries `:protocol = websocket` plus the target scheme, path
/// and authority. The `Connection`, `Upgrade`, `Host` and
/// `Sec-WebSocket-Key` headers of RFC 6455 are not part of HTTP/2 and are
/// dropped; `Sec-WebSocket-Version`, `Sec-WebSocket-Protocol`,
/// `Sec-WebSocket-Extensions` and `Origin` keep their RFC 6455 meaning.
fn build_connect_request(
    url: &Url,
    options: &Options,
    headers: &HeaderMap,
) -> std::result::Result<http::Request<()>, WebSocketError> {
    let scheme = match url.scheme() {
        "ws" => "http",
        "wss" => "https",
        _ => return Err(WebSocketError::InvalidHttpScheme),
    };
    let host = url
        .host_str()
        .ok_or(WebSocketError::InvalidHttpScheme)?
        .to_owned();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "URL has no port"))?;
    let authority = if url.port().is_some() {
        format_authority(&host, Some(port))
    } else {
        format_authority(&host, None)
    };
    let mut path = url.path().to_owned();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    if path.is_empty() {
        path.push('/');
    }

    let uri: Uri = format!("{scheme}://{authority}{path}")
        .parse()
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid target URI: {e}"),
            )
        })?;

    let mut request = http::Request::builder()
        .method(http::Method::CONNECT)
        .uri(uri)
        .extension(Protocol::from_static("websocket"))
        .body(())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;

    // User headers, minus the ones the handshake owns.
    for (name, value) in headers {
        if matches!(
            name.as_str(),
            "connection" | "upgrade" | "host" | "sec-websocket-key" | "sec-websocket-accept"
        ) {
            continue;
        }
        request.headers_mut().insert(name.clone(), value.clone());
    }

    request.headers_mut().insert(
        header::SEC_WEBSOCKET_VERSION,
        header::HeaderValue::from_static("13"),
    );

    if !options.protocols.is_empty() {
        let offered = options.protocols.join(", ");
        let value: header::HeaderValue =
            offered.parse().map_err(|e: header::InvalidHeaderValue| {
                io::Error::new(io::ErrorKind::InvalidInput, e.to_string())
            })?;
        request
            .headers_mut()
            .insert(header::SEC_WEBSOCKET_PROTOCOL, value);
    }

    if let Some(compression) = options.compression.as_ref() {
        let extensions = WebSocketExtensions::from(compression);
        let value: header::HeaderValue =
            extensions
                .to_string()
                .parse()
                .map_err(|e: header::InvalidHeaderValue| {
                    io::Error::new(io::ErrorKind::InvalidInput, e.to_string())
                })?;
        request
            .headers_mut()
            .insert(header::SEC_WEBSOCKET_EXTENSIONS, value);
    }

    Ok(request)
}

/// Maps an `http2` error into a [`WebSocketError`].
fn h2_error(err: http2::Error) -> WebSocketError {
    let message = err.to_string();
    match err.into_io() {
        Some(io) => WebSocketError::IoError(io),
        None => WebSocketError::IoError(io::Error::other(message)),
    }
}

/// Validates the CONNECT response and extracts the negotiation result.
///
/// Any `2xx` status establishes the stream (servers send `200` per RFC 8441
/// §5); accepting the whole class avoids failing against otherwise
/// compliant peers.
fn verify_connect_response(
    response: &http::Response<RecvStream>,
    options: &Options,
) -> std::result::Result<(Option<String>, Option<WebSocketExtensions>), WebSocketError> {
    if !response.status().is_success() {
        return Err(WebSocketError::InvalidStatusCode(
            response.status().as_u16(),
        ));
    }

    let headers = response.headers();
    let protocol = headers
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|h| h.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    if let Some(selected) = protocol.as_deref()
        && !options
            .protocols
            .iter()
            .any(|offered| offered.eq_ignore_ascii_case(selected))
    {
        return Err(WebSocketError::InvalidSecWebSocketProtocol);
    }

    let extensions = headers
        .get(header::SEC_WEBSOCKET_EXTENSIONS)
        .and_then(|h| h.to_str().ok())
        .map(std::str::FromStr::from_str)
        .and_then(std::result::Result::ok);

    Ok((protocol, extensions))
}

async fn connect_h2_priv(opts: H2BuilderOpts) -> std::result::Result<H2WebSocket, WebSocketError> {
    let url = opts.url;
    let options = opts.establish_options.unwrap_or_default();

    let host = url
        .host_str()
        .ok_or(WebSocketError::InvalidHttpScheme)?
        .to_owned();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "URL has no port"))?;

    // The underlying byte stream: direct TCP or tunnelled through a proxy
    // (RFC 8441 §7: a traditional CONNECT tunnel carries the HTTP/2
    // connection when a forward proxy is in use).
    #[cfg(feature = "proxy")]
    let tcp: super::proxy::ProxyStream = match &opts.proxy {
        Some(proxy) => super::proxy::connect_through_proxy(proxy, &host, port).await?,
        None => Box::new(TcpStream::connect(format_authority(&host, Some(port))).await?),
    };
    #[cfg(not(feature = "proxy"))]
    let tcp = TcpStream::connect(format_authority(&host, Some(port))).await?;

    // TLS for `wss://`, negotiating `h2` via ALPN.
    let is_secure = url.scheme() == "wss";
    let mut send_request;
    if is_secure {
        let connector = match opts.connector {
            Some(connector) => connector,
            None => super::tls_connector_for_alpn(vec![b"h2".to_vec()])?,
        };
        let domain = super::ServerName::try_from(host.clone())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid dnsname"))?;
        let tls = connector.connect(domain, tcp).await?;
        if tls.get_ref().1.alpn_protocol() != Some(b"h2".as_slice()) {
            return Err(WebSocketError::AlpnMismatch);
        }
        send_request = drive_connection(tls).await?;
    } else if url.scheme() == "ws" {
        send_request = drive_connection(tcp).await?;
    } else {
        return Err(WebSocketError::InvalidHttpScheme);
    }

    // The server must opt into extended CONNECT before the client may use
    // it (RFC 8441 §3).
    wait_for_extended_connect(&mut send_request).await?;

    // The extended CONNECT handshake (RFC 8441 §5).
    let headers = opts
        .http_builder
        .unwrap_or_else(HttpRequest::builder)
        .body(())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    let request = build_connect_request(&url, &options, headers.headers())?;
    let (response_future, send_stream): (ResponseFuture, SendStream<Bytes>) = send_request
        .send_request(request, false)
        .map_err(h2_error)?;
    let response = response_future.await.map_err(|e| {
        WebSocketError::IoError(io::Error::other(format!("h2 connect failed: {e}")))
    })?;
    let (protocol, extensions) = verify_connect_response(&response, &options)?;
    let recv_stream = response.into_body();

    let stream = H2Stream {
        send: send_stream,
        recv: recv_stream,
        read_buf: BytesMut::new(),
        eof: false,
        shutdown: false,
    };

    Ok(WebSocket::from_h2_stream(
        stream, options, protocol, extensions,
    ))
}

/// Performs the HTTP/2 handshake and spawns the connection driver.
async fn drive_connection<S>(io: S) -> std::result::Result<SendRequest<Bytes>, WebSocketError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (send_request, connection) = http2::client::handshake(io).await.map_err(|e| {
        WebSocketError::IoError(io::Error::other(format!("h2 handshake failed: {e}")))
    })?;

    #[cfg(not(feature = "smol"))]
    tokio::spawn(async move {
        if let Err(err) = connection.await {
            tracing::error!("h2 connection failed: {:?}", err);
        }
    });
    #[cfg(feature = "smol")]
    smol::spawn(async move {
        if let Err(err) = connection.await {
            tracing::error!("h2 connection failed: {:?}", err);
        }
    })
    .detach();

    Ok(send_request)
}

/// Waits until the server enables extended CONNECT.
async fn wait_for_extended_connect(
    send_request: &mut SendRequest<Bytes>,
) -> std::result::Result<(), WebSocketError> {
    tokio::time::timeout(ENABLE_CONNECT_TIMEOUT, async {
        while !send_request.is_extended_connect_protocol_enabled() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .map_err(|_| WebSocketError::ExtendedConnectUnsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options_with_protocols(protocols: &[&str]) -> Options {
        Options::default().with_protocols(protocols.iter().copied())
    }

    #[test]
    fn connect_request_carries_the_extended_connect_pseudos() {
        let url: Url = "wss://server.example.com/chat?room=1".parse().unwrap();
        let options = options_with_protocols(&["chat", "superchat"]);
        let request =
            build_connect_request(&url, &options, &HeaderMap::new()).expect("request built");

        assert_eq!(request.method(), http::Method::CONNECT);
        assert_eq!(
            request.extensions().get::<Protocol>().map(Protocol::as_str),
            Some("websocket")
        );
        assert_eq!(request.uri().scheme_str(), Some("https"));
        assert_eq!(
            request.uri().path_and_query().map(|p| p.as_str()),
            Some("/chat?room=1")
        );
        assert_eq!(
            request
                .headers()
                .get(header::SEC_WEBSOCKET_VERSION)
                .map(|v| v.as_bytes()),
            Some(b"13".as_slice())
        );
        assert_eq!(
            request
                .headers()
                .get(header::SEC_WEBSOCKET_PROTOCOL)
                .and_then(|v| v.to_str().ok()),
            Some("chat, superchat")
        );
    }

    #[test]
    fn connect_request_drops_http1_only_headers() {
        let url: Url = "ws://example.com/socket".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(header::CONNECTION, "upgrade".parse().unwrap());
        headers.insert(header::UPGRADE, "websocket".parse().unwrap());
        headers.insert(header::HOST, "example.com".parse().unwrap());
        headers.insert(header::SEC_WEBSOCKET_KEY, "key".parse().unwrap());
        headers.insert("x-custom", "value".parse().unwrap());

        let request = build_connect_request(&url, &Options::default(), &headers).expect("built");

        assert!(request.headers().get(header::CONNECTION).is_none());
        assert!(request.headers().get(header::UPGRADE).is_none());
        assert!(request.headers().get(header::HOST).is_none());
        assert!(request.headers().get(header::SEC_WEBSOCKET_KEY).is_none());
        assert_eq!(
            request
                .headers()
                .get("x-custom")
                .and_then(|v| v.to_str().ok()),
            Some("value")
        );
    }

    #[test]
    fn connect_request_rejects_non_websocket_schemes() {
        let url: Url = "https://example.com/socket".parse().unwrap();
        assert!(build_connect_request(&url, &Options::default(), &HeaderMap::new()).is_err());
    }

    #[test]
    fn connect_request_brackets_ipv6_authority() {
        let url: Url = "ws://[::1]:8080/chat".parse().unwrap();
        let request =
            build_connect_request(&url, &Options::default(), &HeaderMap::new()).expect("built");
        assert_eq!(
            request.uri().authority().map(|a| a.as_str()),
            Some("[::1]:8080")
        );

        let url: Url = "wss://[2001:db8::1]/chat".parse().unwrap();
        let request =
            build_connect_request(&url, &Options::default(), &HeaderMap::new()).expect("built");
        assert_eq!(
            request.uri().authority().map(|a| a.as_str()),
            Some("[2001:db8::1]")
        );
    }

    /// Drives an `H2Stream` directly over a loopback TCP pair.
    ///
    /// Unlike the framed WebSocket layer (whose growing read buffer rarely
    /// takes the partial-read path), tiny manual reads exercise the stash,
    /// remainder and capacity-release logic on every chunk.
    #[tokio::test]
    async fn h2stream_round_trips_bytes_with_tiny_reads() {
        use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener bound");
        let addr = listener.local_addr().expect("local address");
        let (server_io, client_io) = tokio::join!(
            async {
                let (stream, _) = listener.accept().await.expect("accepted");
                stream
            },
            async {
                tokio::net::TcpStream::connect(addr)
                    .await
                    .expect("connected")
            }
        );

        let mut server_builder = http2::server::Builder::new();
        server_builder.enable_connect_protocol();
        let (client_handshake, server_handshake) = tokio::join!(
            http2::client::handshake(client_io),
            server_builder.handshake::<_, Bytes>(server_io)
        );
        let (mut send_request, client_conn) = client_handshake.expect("client handshake");
        let mut server_conn = server_handshake.expect("server handshake");

        // The server reports when it observes our end-of-stream.
        let saw_eos = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let saw_eos_server = saw_eos.clone();

        tokio::spawn(async move {
            let _ = client_conn.await;
        });
        tokio::spawn(async move {
            while let Some(accepted) = server_conn.accept().await {
                let Ok((request, mut respond)) = accepted else {
                    break;
                };
                // Handle each stream in its own task so the accept loop
                // keeps driving the connection (flushing queued frames).
                let saw_eos = saw_eos_server.clone();
                tokio::spawn(async move {
                    // Release receive capacity and echo everything back in
                    // small pieces.
                    let mut send = respond
                        .send_response(
                            http::Response::builder().status(200).body(()).unwrap(),
                            false,
                        )
                        .expect("response sent");
                    let mut recv = request.into_body();
                    while let Some(Ok(data)) = recv.data().await {
                        let _ = recv.flow_control().release_capacity(data.len());
                        for chunk in data.chunks(1024) {
                            send.send_data(Bytes::copy_from_slice(chunk), false)
                                .expect("echo sent");
                        }
                    }
                    saw_eos.store(true, std::sync::atomic::Ordering::SeqCst);
                });
            }
        });

        // Open a stream from the client side.
        let request = http::Request::builder()
            .method(http::Method::CONNECT)
            .uri("https://example.com/chat")
            .extension(Protocol::from_static("websocket"))
            .body(())
            .expect("request built");
        let (response_future, send_stream) = send_request
            .send_request(request, false)
            .expect("request sent");
        let response = response_future.await.expect("response received");
        assert_eq!(response.status(), http::StatusCode::OK);
        let recv_stream = response.into_body();

        let mut stream = H2Stream {
            send: send_stream,
            recv: recv_stream,
            read_buf: BytesMut::new(),
            eof: false,
            shutdown: false,
        };

        // The Debug representation names the type.
        assert!(format!("{:?}", stream).contains("H2Stream"));

        // Writes report the exact length: a short write would corrupt the
        // byte stream or stall the writer.
        let payload = vec![0xabu8; 100_000];
        let mut written = 0usize;
        while written < payload.len() {
            let n = futures::future::poll_fn(|cx| {
                Pin::new(&mut stream).poll_write(cx, &payload[written..])
            })
            .await
            .expect("write succeeds");
            assert_eq!(n, payload.len() - written, "writes are exact");
            written += n;
        }
        stream.flush().await.expect("flush succeeds");

        // Tiny reads exercise the stash, remainder and capacity-release
        // logic on every chunk. A lost remainder corrupts the stream and
        // fails the equality check below instead of hanging: the whole
        // round trip must complete well within the timeout.
        let mut received = Vec::new();
        let mut byte = [0u8; 7];
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                let n = stream.read(&mut byte).await.expect("read succeeds");
                if n == 0 {
                    break;
                }
                received.extend_from_slice(&byte[..n]);
                if received.len() >= payload.len() {
                    break;
                }
            }
        })
        .await
        .expect("the round trip completes");
        assert_eq!(received, payload);

        // Shutting down sends END_STREAM exactly once: the server observes
        // the end of the stream, and repeating is a no-op.
        stream.shutdown().await.expect("shutdown succeeds");
        stream.shutdown().await.expect("second shutdown is a no-op");
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !saw_eos.load(std::sync::atomic::Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the server observes end-of-stream");
    }
}
