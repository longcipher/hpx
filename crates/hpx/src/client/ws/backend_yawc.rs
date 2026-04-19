//! WebSocket backend using hpx-yawc

use std::{
    borrow::Cow,
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    pin::Pin,
    task::{Context, Poll},
};

use futures_util::{SinkExt, Stream, StreamExt};
use http::{HeaderMap, HeaderName, HeaderValue, Version};

use super::message::{CloseCode, CloseFrame, Message, Utf8Bytes};
use crate::{EmulationFactory, Error, RequestBuilder, header::OrigHeaderMap, proxy::Proxy};

/// Configuration for WebSocket connection.
#[derive(Debug, Clone, Copy)]
pub struct WebSocketConfig {
    /// Maximum message size in bytes.
    pub max_message_size: Option<usize>,
    /// Whether to automatically close the connection when a close frame is received.
    pub auto_close: bool,
    /// Whether to automatically send a pong when a ping frame is received.
    pub auto_pong: bool,
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            max_message_size: None,
            auto_close: true,
            auto_pong: true,
        }
    }
}

/// Wrapper for [`RequestBuilder`] that performs the
/// websocket handshake when sent.
pub struct WebSocketRequestBuilder {
    inner: RequestBuilder,
    unsupported: UnsupportedSettings,
    config: WebSocketConfig,
}

impl WebSocketRequestBuilder {
    /// Creates a new WebSocket request builder.
    pub fn new(inner: RequestBuilder) -> Self {
        Self {
            inner: inner.version(Version::HTTP_11),
            unsupported: UnsupportedSettings::default(),
            config: WebSocketConfig::default(),
        }
    }

    /// Sets a custom WebSocket accept key.
    ///
    /// The yawc backend cannot validate handshake response metadata, so this
    /// option is rejected at send time.
    #[inline]
    pub fn accept_key<K>(mut self, _key: K) -> Self
    where
        K: Into<Cow<'static, str>>,
    {
        self.unsupported.accept_key = true;
        self
    }

    /// Forces the WebSocket connection to use HTTP/2 protocol.
    ///
    /// The yawc backend only performs the standard WebSocket upgrade flow.
    #[inline]
    pub fn force_http2(mut self) -> Self {
        self.unsupported.force_http2 = true;
        self
    }

    /// Sets the websocket subprotocols to request.
    ///
    /// The yawc backend cannot observe the negotiated protocol, so this option
    /// is rejected at send time.
    #[inline]
    pub fn protocols<P>(mut self, _protocols: P) -> Self
    where
        P: IntoIterator,
        P::Item: Into<Cow<'static, str>>,
    {
        self.unsupported.protocols = true;
        self
    }

    /// Sets the websocket max_message_size configuration.
    #[inline]
    pub fn max_message_size(mut self, max_message_size: usize) -> Self {
        self.config.max_message_size = Some(max_message_size);
        self
    }

    /// Sets whether to automatically close the connection when a close frame is received.
    #[inline]
    pub fn auto_close(mut self, auto_close: bool) -> Self {
        self.config.auto_close = auto_close;
        self
    }

    /// Sets whether to automatically send a pong when a ping frame is received.
    #[inline]
    pub fn auto_pong(mut self, auto_pong: bool) -> Self {
        self.config.auto_pong = auto_pong;
        self
    }

    /// Add a `Header` to this Request.
    #[inline]
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<http::Error>,
        HeaderValue: TryFrom<V>,
        <HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        self.inner = self.inner.header(key, value);
        self
    }

    /// Add a set of Headers to the existing ones on this Request.
    #[inline]
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.inner = self.inner.headers(headers);
        self
    }

    /// Set the original headers for this request.
    #[inline]
    pub fn orig_headers(mut self, orig_headers: OrigHeaderMap) -> Self {
        self.inner = self.inner.orig_headers(orig_headers);
        self
    }

    /// Enable or disable client default headers for this request.
    pub fn default_headers(mut self, enable: bool) -> Self {
        self.inner = self.inner.default_headers(enable);
        self
    }

    /// Enable HTTP authentication.
    #[inline]
    pub fn auth<V>(mut self, value: V) -> Self
    where
        HeaderValue: TryFrom<V>,
        <HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        self.inner = self.inner.auth(value);
        self
    }

    /// Enable HTTP basic authentication.
    #[inline]
    pub fn basic_auth<U, P>(mut self, username: U, password: Option<P>) -> Self
    where
        U: fmt::Display,
        P: fmt::Display,
    {
        self.inner = self.inner.basic_auth(username, password);
        self
    }

    /// Enable HTTP bearer authentication.
    #[inline]
    pub fn bearer_auth<T>(mut self, token: T) -> Self
    where
        T: fmt::Display,
    {
        self.inner = self.inner.bearer_auth(token);
        self
    }

    /// Modify the query string of the URI.
    #[inline]
    #[cfg(feature = "query")]
    #[cfg_attr(docsrs, doc(cfg(feature = "query")))]
    pub fn query<T: serde::Serialize + ?Sized>(mut self, query: &T) -> Self {
        self.inner = self.inner.query(query);
        self
    }

    /// Set the proxy for this request.
    ///
    /// The yawc backend does not wire through proxy settings, so this option is
    /// rejected at send time.
    #[inline]
    pub fn proxy(mut self, _proxy: Proxy) -> Self {
        self.unsupported.proxy = true;
        self
    }

    /// Set the local address for this request.
    #[inline]
    pub fn local_address<V>(mut self, _local_address: V) -> Self
    where
        V: Into<Option<IpAddr>>,
    {
        self.unsupported.local_address = true;
        self
    }

    /// Set the local addresses for this request.
    #[inline]
    pub fn local_addresses<V4, V6>(mut self, _ipv4: V4, _ipv6: V6) -> Self
    where
        V4: Into<Option<Ipv4Addr>>,
        V6: Into<Option<Ipv6Addr>>,
    {
        self.unsupported.local_addresses = true;
        self
    }

    /// Set the interface for this request.
    #[inline]
    #[cfg(any(
        target_os = "android",
        target_os = "fuchsia",
        target_os = "illumos",
        target_os = "ios",
        target_os = "linux",
        target_os = "macos",
        target_os = "solaris",
        target_os = "tvos",
        target_os = "visionos",
        target_os = "watchos",
    ))]
    #[cfg_attr(
        docsrs,
        doc(cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "illumos",
            target_os = "ios",
            target_os = "linux",
            target_os = "macos",
            target_os = "solaris",
            target_os = "tvos",
            target_os = "visionos",
            target_os = "watchos",
        )))
    )]
    pub fn interface<I>(mut self, _interface: I) -> Self
    where
        I: Into<std::borrow::Cow<'static, str>>,
    {
        self.unsupported.interface = true;
        self
    }

    /// Set the emulation for this request.
    ///
    /// The yawc backend does not wire through the transport hooks required to
    /// apply browser emulation settings.
    #[inline]
    pub fn emulation<P>(mut self, _factory: P) -> Self
    where
        P: EmulationFactory,
    {
        self.unsupported.emulation = true;
        self
    }

    /// Sends the request and returns a [`WebSocketResponse`].
    pub async fn send(self) -> Result<WebSocketResponse, Error> {
        self.unsupported.check()?;

        let (_client, request) = self.inner.build_split();
        let request = request?;
        let uri = request.uri().clone();

        let url: url::Url = uri
            .to_string()
            .parse()
            .map_err(|e: url::ParseError| Error::builder(e))?;

        let mut http_builder = hpx_yawc::HttpRequest::builder();
        for (name, value) in request.headers() {
            let value = value.to_str().map_err(|_| {
                Error::upgrade(format!("unsupported non-UTF-8 header value for {name}"))
            })?;
            http_builder = http_builder.header(name.as_str(), value);
        }

        let mut options = hpx_yawc::Options::default();
        if let Some(max_size) = self.config.max_message_size {
            options = options.with_max_payload_read(max_size);
        }

        let ws = hpx_yawc::WebSocket::connect(url)
            .with_options(options)
            .with_request(http_builder)
            .await
            .map_err(|e| Error::upgrade(e.to_string()))?;

        Ok(WebSocketResponse { ws: Some(ws) })
    }
}

/// The server's response to the websocket upgrade request.
///
/// The yawc backend does not expose handshake metadata such as the negotiated
/// HTTP status, version, or response headers.
pub struct WebSocketResponse {
    ws: Option<hpx_yawc::TcpWebSocket>,
}

impl fmt::Debug for WebSocketResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebSocketResponse")
            .field("connected", &self.ws.is_some())
            .finish()
    }
}

impl WebSocketResponse {
    /// Turns the response into a websocket.
    pub async fn into_websocket(mut self) -> Result<WebSocket, Error> {
        let ws = self
            .ws
            .take()
            .ok_or_else(|| Error::upgrade("WebSocket already consumed"))?;
        Ok(WebSocket { inner: ws })
    }
}

/// Convert a yawc Frame to our Message type.
fn frame_to_message(frame: hpx_yawc::Frame) -> Message {
    let (opcode, _is_fin, payload) = frame.into_parts();
    match opcode {
        hpx_yawc::OpCode::Text => {
            let s = String::from_utf8_lossy(&payload).to_string();
            Message::Text(Utf8Bytes::from(s))
        }
        hpx_yawc::OpCode::Binary => Message::Binary(payload),
        hpx_yawc::OpCode::Ping => Message::Ping(payload),
        hpx_yawc::OpCode::Pong => Message::Pong(payload),
        hpx_yawc::OpCode::Close => {
            if payload.len() >= 2 {
                let code = u16::from_be_bytes([payload[0], payload[1]]);
                let reason = String::from_utf8_lossy(&payload[2..]).to_string();
                Message::Close(Some(CloseFrame {
                    code: CloseCode(code),
                    reason: Utf8Bytes::from(reason),
                }))
            } else {
                Message::Close(None)
            }
        }
        hpx_yawc::OpCode::Continuation => Message::Binary(payload),
    }
}

/// Convert our Message type to a yawc Frame.
fn message_to_frame(msg: Message) -> hpx_yawc::Frame {
    match msg {
        Message::Text(text) => hpx_yawc::Frame::text(bytes::Bytes::from(text.as_str().to_owned())),
        Message::Binary(data) => hpx_yawc::Frame::binary(data),
        Message::Ping(data) => hpx_yawc::Frame::ping(data),
        Message::Pong(data) => hpx_yawc::Frame::pong(data),
        Message::Close(Some(close)) => {
            let yawc_code: hpx_yawc::close::CloseCode = close.code.0.into();
            hpx_yawc::Frame::close(yawc_code, close.reason.as_bytes())
        }
        Message::Close(None) => hpx_yawc::Frame::close(hpx_yawc::close::CloseCode::Normal, []),
    }
}

/// A websocket connection using the yawc backend.
pub struct WebSocket {
    inner: hpx_yawc::TcpWebSocket,
}

impl WebSocket {
    /// Receive another message.
    ///
    /// Returns `None` if the stream has closed.
    #[inline]
    pub async fn recv(&mut self) -> Option<Result<Message, Error>> {
        self.inner
            .next()
            .await
            .map(|frame| Ok(frame_to_message(frame)))
    }

    /// Send a message.
    #[inline]
    pub async fn send(&mut self, msg: Message) -> Result<(), Error> {
        let frame = message_to_frame(msg);
        self.inner
            .send(frame)
            .await
            .map_err(|e| Error::upgrade(e.to_string()))
    }

    /// Closes the connection with a given code and (optional) reason.
    pub async fn close<C, R>(mut self, code: C, reason: R) -> Result<(), Error>
    where
        C: Into<CloseCode>,
        R: Into<Utf8Bytes>,
    {
        let code = code.into();
        let reason = reason.into();
        let yawc_code: hpx_yawc::close::CloseCode = code.0.into();
        let frame = hpx_yawc::Frame::close(yawc_code, reason.as_bytes());
        self.inner
            .send(frame)
            .await
            .map_err(|e| Error::upgrade(e.to_string()))
    }

    /// Split the WebSocket into a reader and a writer.
    pub fn split(self) -> (WebSocketWrite, WebSocketRead) {
        let (sink, stream) = self.inner.split();
        (
            WebSocketWrite { inner: sink },
            WebSocketRead { inner: stream },
        )
    }
}

/// A WebSocket reader using the yawc backend.
pub struct WebSocketRead {
    inner: futures_util::stream::SplitStream<hpx_yawc::TcpWebSocket>,
}

impl Stream for WebSocketRead {
    type Item = Result<Message, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Ready(Some(frame)) => Poll::Ready(Some(Ok(frame_to_message(frame)))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl WebSocketRead {
    /// Receive another message.
    pub async fn recv(&mut self) -> Option<Result<Message, Error>> {
        self.inner
            .next()
            .await
            .map(|frame| Ok(frame_to_message(frame)))
    }
}

/// A WebSocket writer using the yawc backend.
pub struct WebSocketWrite {
    inner: futures_util::stream::SplitSink<hpx_yawc::TcpWebSocket, hpx_yawc::Frame>,
}

impl WebSocketWrite {
    /// Send a message.
    pub async fn send(&mut self, msg: Message) -> Result<(), Error> {
        let frame = message_to_frame(msg);
        self.inner
            .send(frame)
            .await
            .map_err(|e| Error::upgrade(e.to_string()))
    }

    /// Closes the connection with a given code and (optional) reason.
    pub async fn close<C, R>(mut self, code: C, reason: R) -> Result<(), Error>
    where
        C: Into<CloseCode>,
        R: Into<Utf8Bytes>,
    {
        let code = code.into();
        let reason = reason.into();
        let yawc_code: hpx_yawc::close::CloseCode = code.0.into();
        let frame = hpx_yawc::Frame::close(yawc_code, reason.as_bytes());
        self.inner
            .send(frame)
            .await
            .map_err(|e| Error::upgrade(e.to_string()))
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct UnsupportedSettings {
    accept_key: bool,
    force_http2: bool,
    protocols: bool,
    proxy: bool,
    local_address: bool,
    local_addresses: bool,
    interface: bool,
    emulation: bool,
}

impl UnsupportedSettings {
    fn check(&self) -> Result<(), Error> {
        let mut unsupported = Vec::new();

        if self.accept_key {
            unsupported.push("accept_key");
        }
        if self.force_http2 {
            unsupported.push("force_http2");
        }
        if self.protocols {
            unsupported.push("protocols");
        }
        if self.proxy {
            unsupported.push("proxy");
        }
        if self.local_address {
            unsupported.push("local_address");
        }
        if self.local_addresses {
            unsupported.push("local_addresses");
        }
        if self.interface {
            unsupported.push("interface");
        }
        if self.emulation {
            unsupported.push("emulation");
        }

        if unsupported.is_empty() {
            Ok(())
        } else {
            Err(Error::upgrade(format!(
                "unsupported yawc websocket settings: {}",
                unsupported.join(", ")
            )))
        }
    }
}
